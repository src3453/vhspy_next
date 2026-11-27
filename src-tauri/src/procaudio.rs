//! ウィンドウのアプリの音声を取り込む (WASAPI のプロセスループバック、Windows 10 2004 以降)。
//!
//! 取り込んだ音声はテープの音声処理に通し、既定の出力デバイスで鳴らす (モニター) と同時に、
//! 録画中なら一時 WAV に書く。録画の終わりに映像と多重化する。

use crate::engine::Shared;
use crate::ffmpeg::{AUDIO_RATE, write_wav_header};
use std::collections::VecDeque;
use std::io::{BufWriter, Seek, Write};
use std::mem::ManuallyDrop;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use vhs_core::audio::AudioProcessor;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, AUDIOCLIENT_ACTIVATION_PARAMS,
    AUDIOCLIENT_ACTIVATION_PARAMS_0, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
    AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS, ActivateAudioInterfaceAsync,
    IActivateAudioInterfaceAsyncOperation, IActivateAudioInterfaceCompletionHandler,
    IActivateAudioInterfaceCompletionHandler_Impl, IAudioCaptureClient, IAudioClient,
    IAudioRenderClient, IMMDeviceEnumerator, MMDeviceEnumerator,
    PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
    WAVEFORMATEX, eConsole, eRender,
};
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
use windows::Win32::System::Com::{
    BLOB, CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::Win32::System::Variant::VT_BLOB;
use windows::core::{HRESULT, IUnknown, Interface, implement};

/// 48 kHz / 2ch / f32
fn format() -> WAVEFORMATEX {
    WAVEFORMATEX {
        wFormatTag: 3, // WAVE_FORMAT_IEEE_FLOAT
        nChannels: 2,
        nSamplesPerSec: AUDIO_RATE,
        nAvgBytesPerSec: AUDIO_RATE * 8,
        nBlockAlign: 8,
        wBitsPerSample: 32,
        cbSize: 0,
    }
}

/// 録画中の音声 (一時 WAV)
pub struct WavSink {
    out: BufWriter<std::fs::File>,
    written: u64,
    /// 音声処理の遅れを打ち消すため頭で捨てるサンプル数 (インターリーブ)
    skip: usize,
    pub path: PathBuf,
}

impl WavSink {
    pub fn create(path: PathBuf) -> Result<Self, String> {
        let file =
            std::fs::File::create(&path).map_err(|e| format!("一時ファイルを作れません: {e}"))?;
        let mut out = BufWriter::new(file);
        write_wav_header(&mut out, 0).map_err(|e| e.to_string())?;
        Ok(Self {
            out,
            written: 0,
            skip: usize::MAX,
            path,
        })
    }

    fn write(&mut self, s: &[f32], latency: usize) {
        if self.skip == usize::MAX {
            self.skip = latency * 2;
        }
        let n = self.skip.min(s.len());
        self.skip -= n;
        for v in &s[n..] {
            let _ = self.out.write_all(&v.to_le_bytes());
        }
        self.written += (s.len() - n) as u64;
    }

    /// ヘッダを書き直して閉じる。1 サンプルもなければ None
    pub fn finish(self) -> Result<Option<PathBuf>, String> {
        let mut file = self.out.into_inner().map_err(|e| e.to_string())?;
        if self.written == 0 {
            drop(file);
            let _ = std::fs::remove_file(&self.path);
            return Ok(None);
        }
        file.seek(std::io::SeekFrom::Start(0))
            .map_err(|e| e.to_string())?;
        write_wav_header(&mut file, self.written * 4).map_err(|e| e.to_string())?;
        Ok(Some(self.path))
    }
}

/// プロセスの音声の取り込み。捨てると止まる
pub struct LiveAudio {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    /// Some のあいだ処理した音声を書き込む (録画)
    pub sink: Arc<Mutex<Option<WavSink>>>,
}

impl LiveAudio {
    /// プロセス `pid` (と子プロセス) の音声を取り込み始める
    pub fn start(pid: u32, monitor: bool, shared: Arc<Shared>) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let sink: Arc<Mutex<Option<WavSink>>> = Arc::new(Mutex::new(None));
        let (rtx, rrx) = channel();
        let (s2, k2) = (stop.clone(), sink.clone());
        let thread = std::thread::Builder::new()
            .name("vhs-audio".into())
            .spawn(move || {
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                }
                run(pid, monitor, &shared, &s2, &k2, rtx);
                unsafe { CoUninitialize() };
            })
            .map_err(|e| e.to_string())?;
        match rrx.recv() {
            Ok(Ok(())) => Ok(Self {
                stop,
                thread: Some(thread),
                sink,
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err("音声スレッドが止まりました".into()),
        }
    }
}

impl Drop for LiveAudio {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Activated(Mutex<Sender<()>>);

impl IActivateAudioInterfaceCompletionHandler_Impl for Activated_Impl {
    fn ActivateCompleted(
        &self,
        _op: windows::core::Ref<IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        let _ = self.0.lock().unwrap().send(());
        Ok(())
    }
}

/// プロセスループバックの IAudioClient を作る
fn activate_loopback(pid: u32) -> Result<IAudioClient, String> {
    let params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    // BLOB は params を指すだけなので PropVariantClear させない (ManuallyDrop)
    let pv = ManuallyDrop::new(PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 {
                    blob: BLOB {
                        cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                        pBlobData: &params as *const _ as *mut u8,
                    },
                },
            }),
        },
    });
    let (tx, rx) = channel();
    let handler: IActivateAudioInterfaceCompletionHandler = Activated(Mutex::new(tx)).into();
    let op = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&*pv as *const PROPVARIANT),
            &handler,
        )
    }
    .map_err(|e| format!("音声の取り込みを開始できません (Windows 10 2004 以降が必要です): {e}"))?;
    rx.recv_timeout(Duration::from_secs(5))
        .map_err(|_| "音声の取り込みの開始が完了しません".to_string())?;
    let mut hr = HRESULT(0);
    let mut unk: Option<IUnknown> = None;
    unsafe { op.GetActivateResult(&mut hr, &mut unk) }.map_err(|e| e.to_string())?;
    hr.ok()
        .map_err(|e| format!("音声の取り込みを開始できません: {e}"))?;
    unk.ok_or("音声の取り込みを開始できません")?
        .cast()
        .map_err(|e| e.to_string())
}

struct Capture {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    event: HANDLE,
}

impl Drop for Capture {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
            let _ = CloseHandle(self.event);
        }
    }
}

fn open_capture(pid: u32) -> Result<Capture, String> {
    let client = activate_loopback(pid)?;
    let wfx = format();
    let e = |e: windows::core::Error| format!("音声の取り込みを開始できません: {e}");
    unsafe {
        client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK
                    | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                    | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
                2_000_000, // 200 ms
                0,
                &wfx,
                None,
            )
            .map_err(e)?;
        let event = CreateEventW(None, false, false, None).map_err(e)?;
        client.SetEventHandle(event).map_err(e)?;
        let capture: IAudioCaptureClient = client.GetService().map_err(e)?;
        client.Start().map_err(e)?;
        Ok(Capture {
            client,
            capture,
            event,
        })
    }
}

struct Render {
    client: IAudioClient,
    render: IAudioRenderClient,
    frames: u32,
}

impl Drop for Render {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
        }
    }
}

/// 既定の出力デバイス
fn open_render() -> windows::core::Result<Render> {
    unsafe {
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let dev = en.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let client: IAudioClient = dev.Activate(CLSCTX_ALL, None)?;
        let wfx = format();
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            1_000_000, // 100 ms
            0,
            &wfx,
            None,
        )?;
        let render: IAudioRenderClient = client.GetService()?;
        let frames = client.GetBufferSize()?;
        client.Start()?;
        Ok(Render {
            client,
            render,
            frames,
        })
    }
}

impl Render {
    /// 鳴らせるだけ `queue` (インターリーブ) から書き込む
    fn feed(&self, queue: &mut VecDeque<f32>) -> windows::core::Result<()> {
        unsafe {
            let pad = self.client.GetCurrentPadding()?;
            let n = (self.frames - pad).min((queue.len() / 2) as u32);
            if n == 0 {
                return Ok(());
            }
            let p = self.render.GetBuffer(n)? as *mut f32;
            let dst = std::slice::from_raw_parts_mut(p, n as usize * 2);
            for (d, s) in dst.iter_mut().zip(queue.drain(..n as usize * 2)) {
                *d = s;
            }
            self.render.ReleaseBuffer(n, 0)
        }
    }
}

fn run(
    pid: u32,
    monitor: bool,
    shared: &Shared,
    stop: &AtomicBool,
    sink: &Mutex<Option<WavSink>>,
    ready: Sender<Result<(), String>>,
) {
    let cap = match open_capture(pid) {
        Ok(c) => c,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let render = if monitor {
        match open_render() {
            Ok(r) => Some(r),
            Err(e) => {
                eprintln!("[audio] 出力デバイスを開けません: {e}");
                None
            }
        }
    } else {
        None
    };
    let _ = ready.send(Ok(()));

    let rate = AUDIO_RATE as usize;
    let mut seen = 0u64;
    let mut proc: Option<AudioProcessor> = None;
    let mut latency = 0usize;
    let mut queue: VecDeque<f32> = VecDeque::new();
    let mut buf: Vec<f32> = Vec::new();
    let start = Instant::now();
    let mut produced = 0usize; // フレーム
    let mut primed = false;

    while !stop.load(Ordering::Relaxed) {
        unsafe { WaitForSingleObject(cap.event, 20) };

        // パラメータが変わったら作り直す
        let v = shared.params_version();
        if v != seen {
            seen = v;
            let p = shared.params.lock().unwrap().clone();
            proc = p.audio.enabled.then(|| AudioProcessor::new(&p, AUDIO_RATE));
            latency = proc.as_ref().map_or(0, AudioProcessor::latency);
        }

        buf.clear();
        loop {
            let n = match unsafe { cap.capture.GetNextPacketSize() } {
                Ok(n) => n,
                Err(e) => {
                    shared.stats.lock().unwrap().error =
                        Some(format!("音声の取り込みが止まりました: {e}"));
                    return;
                }
            };
            if n == 0 {
                break;
            }
            let mut data = std::ptr::null_mut();
            let mut frames = 0u32;
            let mut flags = 0u32;
            if unsafe {
                cap.capture
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
            }
            .is_err()
            {
                break;
            }
            let len = frames as usize * 2;
            if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                buf.resize(buf.len() + len, 0.0);
            } else {
                let s = unsafe { std::slice::from_raw_parts(data as *const f32, len) };
                buf.extend_from_slice(s);
            }
            unsafe {
                let _ = cap.capture.ReleaseBuffer(frames);
            }
        }
        // 対象のアプリが鳴っていないとパケットが来ないことがある。
        // 録画の音声が映像より短くならないよう、時計に対して 100 ms 以上遅れたら無音で埋める
        let expected = (start.elapsed().as_secs_f64() * rate as f64) as usize;
        let have = produced + buf.len() / 2;
        if expected > have + rate / 10 {
            buf.resize(buf.len() + (expected - have - rate / 20) * 2, 0.0);
        }
        if buf.is_empty() {
            continue;
        }
        produced += buf.len() / 2;

        if let Some(p) = &mut proc {
            p.process(&mut buf);
        }
        if let Some(s) = sink.lock().unwrap().as_mut() {
            s.write(&buf, latency);
        }
        if let Some(r) = &render {
            queue.extend(buf.iter().copied());
            // 遅れが溜まったら捨てて詰める (取り込みと出力の時計のずれ)
            let max = rate / 5 * 2;
            if queue.len() > max {
                let drop_n = queue.len() - rate / 20 * 2;
                queue.drain(..drop_n);
            }
            // 最初は少し溜めてから鳴らす (途切れ防止)
            if !primed && queue.len() < rate / 50 * 2 {
                continue;
            }
            primed = true;
            let _ = r.feed(&mut queue);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 音を出しているプロセスが必要 (VHS_TEST_RAW=1 で音声処理を切る): `VHS_TEST_PID=1234 cargo test -p vhs-next procaudio -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn captures_process_audio() {
        let pid: u32 = std::env::var("VHS_TEST_PID")
            .expect("VHS_TEST_PID")
            .parse()
            .unwrap();
        let engine = crate::engine::Engine::start();
        if std::env::var_os("VHS_TEST_RAW").is_some() {
            let mut p = vhs_core::Params::default();
            p.audio.enabled = false;
            engine.set_params(p);
        }
        let a = LiveAudio::start(pid, false, engine.shared.clone()).unwrap();
        let path = std::env::temp_dir().join("vhs_test_live.wav");
        *a.sink.lock().unwrap() = Some(WavSink::create(path.clone()).unwrap());
        std::thread::sleep(Duration::from_secs(3));
        let sink = a.sink.lock().unwrap().take().unwrap();
        drop(a);
        let p = sink.finish().unwrap().expect("音声なし");
        let bytes = std::fs::read(&p).unwrap();
        let samples: Vec<f32> = bytes[44..]
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        let secs = samples.len() as f32 / 2.0 / AUDIO_RATE as f32;
        let peak = samples.iter().fold(0f32, |m, v| m.max(v.abs()));
        println!("{secs:.2} s, peak {peak:.3}, {}", p.display());
        assert!((secs - 3.0).abs() < 0.3, "{secs}");
    }
}
