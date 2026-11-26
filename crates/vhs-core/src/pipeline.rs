//! 全体のパイプライン: RGB → コンポジット → テープ → 受像機 → RGBA。

use crate::adjust::InputAdjust;
use crate::camera::{CameraInput, CameraSim};
use crate::capture::{CaptureSim, CodecStats};
use crate::decoder::Decoder;
use crate::deinterlace::Deinterlacer;
use crate::dv::{DvSim, DvStats};
use crate::encoder::{encode_field_full, encode_field_yc_with_signal};
use crate::frame::{Field, LinearFrame, RgbFrame, RgbaFrame, VideoFrame};
use crate::osd::Osd;
use crate::params::{Connection, OsdMode, OsdParams, Params};
use crate::rf::RfSim;
use crate::tape::TapeSim;
use crate::tbc::Tbc;
use crate::timing::Standard;
use rayon::prelude::*;
use std::time::{Duration, Instant};

/// 放送受信側の RF の乱数系列をデッキ→テレビ側と分けるための値
const RF_BROADCAST_SEED: u64 = 0x5246_4252_4f41_4443;

#[derive(Clone, Copy, Debug, Default)]
pub struct StageTimings {
    /// 入力映像の画質調整
    pub input: Duration,
    pub camera: Duration,
    pub encode: Duration,
    pub tape: Duration,
    pub rf: Duration,
    pub decode: Duration,
    pub capture: Duration,
}

impl StageTimings {
    pub fn total(&self) -> Duration {
        self.input + self.camera + self.encode + self.tape + self.rf + self.decode + self.capture
    }
}

pub struct Pipeline {
    params: Params,
    /// 入力映像の画質調整 (カメラの有無に関わらず先頭で掛ける)
    adjust: InputAdjust,
    camera: CameraSim,
    field: Field,
    tape: TapeSim,
    /// DV の記録・再生 (デジタルのフォーマットのとき、アナログのテープの代わり)
    dv: DvSim,
    tbc: Tbc,
    /// 放送受信 (テープの前) の RF
    rf_broadcast: RfSim,
    /// デッキ → テレビ (テープの後) の RF
    rf: RfSim,
    decoder: Decoder,
    deinterlacer: Deinterlacer,
    capture: CaptureSim,
    osd: Osd,
    /// デッキのメニュー表示 (キャラクタジェネレータ)。テープ再生後のコンポジット信号に重畳する
    menu: Osd,
    /// メニュー表示の内容 (None なら表示しない)
    menu_params: Option<OsdParams>,
    /// エンコーダに渡す映像信号の作業領域 (カメラなしの入力、OSD の焼き込み)
    video: VideoFrame,
    output: RgbaFrame,
    field_index: u64,
    pub timings: StageTimings,
}

impl Pipeline {
    pub fn new(params: Params) -> Self {
        let std = params.tv();
        let (w, h) = std.image_size();
        let mut camera = CameraSim::default();
        camera.set_field_rate(std.field_rate);
        Self {
            params,
            adjust: InputAdjust::default(),
            camera,
            field: Field::new(std),
            tape: TapeSim::new(std),
            dv: DvSim::new(std),
            tbc: Tbc::new(std),
            rf_broadcast: RfSim::default(),
            rf: RfSim::default(),
            decoder: Decoder::new(std),
            deinterlacer: Deinterlacer::new(w, h),
            capture: CaptureSim::default(),
            osd: Osd::new(std),
            menu: Osd::with_vcr_face(std),
            menu_params: None,
            video: VideoFrame::new(w, h),
            output: RgbaFrame::new(w, h),
            field_index: 0,
            timings: StageTimings::default(),
        }
    }

    pub fn params(&self) -> &Params {
        &self.params
    }

    /// パラメータを差し替える。テレビ方式 (228 周期ラインの切り替えを含む) が変わったときは内部状態を作り直す
    /// (画像サイズが変わるので、入力も [`Pipeline::input_size`] に合わせ直すこと)。
    pub fn set_params(&mut self, params: Params) {
        if !std::ptr::eq(params.tv(), self.field.std) {
            let index = self.field_index;
            let menu = self.menu_params.take();
            *self = Self::new(params);
            self.field_index = index;
            self.menu_params = menu;
        } else {
            self.params = params;
        }
    }

    /// デッキのメニュー表示 (OSD マークアップ) を設定する。None で消す。
    /// 内蔵の VCR 風ドットフォントで描き、テープ再生後のコンポジット信号に重畳する。
    /// 座標系・タグは通常の OSD と同じ ([`crate::osd::markup`])。
    pub fn set_menu(&mut self, markup: Option<String>) {
        self.menu_params = markup.map(|markup| OsdParams { enabled: true, markup, scale: 2, ..OsdParams::default() });
    }

    /// 現在のテレビ方式
    pub fn standard(&self) -> &'static Standard {
        self.field.std
    }

    /// 入力画像の解像度 (幅, 高さ)。これに合わせておくと内部リサイズが不要になる
    pub fn input_size(&self) -> (usize, usize) {
        self.standard().image_size()
    }

    /// 出力画像の解像度 (幅, 高さ)。表示時に 4:3 へ引き伸ばす (NTSC のピクセルアスペクト比 ≒ 0.849)
    pub fn output_size(&self) -> (usize, usize) {
        self.standard().image_size()
    }

    /// 最終出力 (キャプチャが有効ならキャプチャ後の画像)
    pub fn output(&self) -> &RgbaFrame {
        match self.capture.output() {
            Some(o) if self.params.capture.enabled => o,
            _ => &self.output,
        }
    }

    /// 直前のフレームの圧縮の状態 (キャプチャ有効時)
    pub fn codec_stats(&self) -> Option<CodecStats> {
        (self.params.capture.enabled && self.params.capture.codec).then(|| self.capture.codec_stats())
    }

    /// 直前のフレームの DV の符号化・エラーの状態 (DV のフォーマットのとき)
    pub fn dv_stats(&self) -> Option<DvStats> {
        (self.params.tape.enabled && self.params.tape.format.is_digital()).then_some(self.dv.stats)
    }

    pub fn field_index(&self) -> u64 {
        self.field_index
    }

    /// 1 フィールドぶん処理する。入力 1 枚 = 1 フィールド (NTSC なら 59.94 Hz)。
    pub fn process_field(&mut self, input: &RgbFrame) -> &RgbaFrame {
        let (w, h) = self.input_size();
        let resized;
        let input = if input.width == w && input.height == h {
            input
        } else {
            resized = input.resized(w, h);
            &resized
        };
        if InputAdjust::is_identity(&self.params.input) {
            self.run(CameraInput::Rgb(input));
            return self.output();
        }
        // run が &mut self を取るので、調整器を一時的に取り出す
        let t = Instant::now();
        let mut adjust = std::mem::take(&mut self.adjust);
        let adjusted = adjust.process_rgb(input, &self.params.input);
        let d = t.elapsed();
        self.run(CameraInput::Rgb(adjusted));
        self.adjust = adjust;
        self.timings.input = d;
        self.output()
    }

    /// シーンリニアな HDR 映像 (3DCG のレンダリング結果など) を 1 フィールドぶん処理する。
    /// 白を超える光も撮像管カメラ (コメットテール・焼き付き・ニー) にそのまま届く。
    /// カメラが無効なときは 0..1 で切ってガンマ 1/2.2 の 8 bit にしてから流す。
    pub fn process_field_linear(&mut self, input: &LinearFrame) -> &RgbaFrame {
        let (w, h) = self.input_size();
        let resized;
        let input = if input.width == w && input.height == h {
            input
        } else {
            resized = input.resized(w, h);
            &resized
        };
        if InputAdjust::is_identity(&self.params.input) {
            self.run(CameraInput::Linear(input));
            return self.output();
        }
        let t = Instant::now();
        let mut adjust = std::mem::take(&mut self.adjust);
        let adjusted = adjust.process_linear(input, &self.params.input);
        let d = t.elapsed();
        self.run(CameraInput::Linear(adjusted));
        self.adjust = adjust;
        self.timings.input = d;
        self.output()
    }

    fn run(&mut self, src: CameraInput) -> &RgbaFrame {
        self.field.set_index(self.field_index);
        let p = &self.params;
        let osd = (p.osd.enabled && self.osd.update(&p.osd, self.field_index)).then_some(p.osd.mode);
        let opacity = p.osd.opacity.clamp(0.0, 1.0);
        let menu = self.menu_params.as_ref().is_some_and(|m| self.menu.update(m, self.field_index));

        let tc = Instant::now();
        // 撮像管カメラ (OSD の日付などはカメラの後段で焼き込まれる)
        let burn = osd == Some(OsdMode::Tape);
        if p.camera.enabled {
            let cam = self.camera.process(src, &p.camera, p.seed, self.field_index);
            if burn {
                self.video.data.copy_from_slice(&cam.data);
            }
        } else {
            match src {
                CameraInput::Rgb(f) => self.video.copy_from_rgb(f),
                CameraInput::Linear(f) => self.video.copy_from_linear(f),
            }
        }
        if burn {
            self.osd.burn_into(&mut self.video, opacity);
        }
        let input = match self.camera.output() {
            Some(cam) if p.camera.enabled && !burn => cam,
            _ => &self.video,
        };
        // DV: カメラの映像をデジタルのまま記録・再生する (アナログのテープ処理の代わり)
        let digital = p.tape.enabled && p.tape.format.is_digital();
        let t0 = Instant::now();
        let input = if digital {
            self.dv.process(input, self.field.parity, self.field.frame, &p.dv, p.seed)
        } else {
            input
        };
        let t_dv = Instant::now();
        let (mut t1, mut t1r, mut t2, mut t3) = (t_dv, t_dv, t_dv, t_dv);
        // 画面に描いた行の偶奇 (受像機が垂直同期から決める。画面が流れているとフィールドの偶奇と一致しない)
        let mut shown_parity = self.field.parity;
        if p.encoder.enabled {
            // S 映像はテープの再生出力を受像機へ直接つなぐときだけ (テープなし・テープ後の RF 接続はコンポジット)
            let svideo = p.output.connection == Connection::SVideo
                && p.tape.enabled
                && !(p.rf.enabled && p.rf.placement.after_tape());
            if digital {
                // DV デッキはデジタルの映像から直接 Y/C (またはコンポジット) を作る
                self.field.set_separate(svideo);
                encode_field_yc_with_signal(input, &mut self.field, &p.encoder, &p.signal, p.seed);
            } else {
                // 帰線区間の VBI データ・試験信号も送出側で載せる
                self.field.set_separate(false);
                encode_field_full(input, &mut self.field, &p.encoder, &p.signal, &p.vbi, p.seed);
            }
            t1 = Instant::now();
            // 放送受信: 受信時の劣化ごとテープに記録される (デッキ→テレビの RF とは独立した雑音)。
            // DV はカメラの映像を直接記録するので対象外
            if p.rf.enabled && p.rf.placement.before_tape() && !digital {
                self.rf_broadcast.process(&mut self.field, &p.rf, p.seed ^ RF_BROADCAST_SEED);
            }
            t1r = Instant::now();
            // アナログのテープは再生時に Y/C を分けて出す
            self.field.set_separate(svideo);
            if p.tape.enabled && !digital {
                self.tape.process(&mut self.field, &p.tape, &p.transport, p.seed);
                // 再生機の TBC (デッキの OSD はその後で重ねる)
                if p.tbc.enabled {
                    self.tbc.process(&mut self.field, &p.tbc, p.seed);
                }
            }
            // デッキ OSD は再生信号に同期してテープ後の出力へ混ぜる。後段の RF・復調処理は受ける。
            if osd == Some(OsdMode::Composite) {
                self.osd.insert_composite(&mut self.field, opacity);
            }
            if menu {
                self.menu.insert_composite(&mut self.field, 1.0);
            }
            t2 = Instant::now();
            if p.rf.enabled && p.rf.placement.after_tape() {
                self.rf.process(&mut self.field, &p.rf, p.seed);
            }
            t3 = Instant::now();
            shown_parity = self.decoder.decode_field(
                &self.field,
                &p.decoder,
                p.tape.enabled && !digital,
                &mut self.output,
            );
        } else {
            // コンポジットを通さない: 映像をデジタルのまま出す (DV なら FireWire 取り込み)
            write_field(input, self.field.parity, &mut self.output);
        }
        self.deinterlacer.process_field(
            &mut self.output,
            self.field_index,
            shown_parity,
            p.output.deinterlace,
            &p.output.deinterlace_params,
        );
        let t4 = Instant::now();
        // 受像機の字幕デコーダ (画面に重ねる文字なので、インターレース解除の後に描く)
        if p.encoder.enabled && p.decoder.caption_display {
            self.decoder.captions().render(&mut self.output, self.field_index);
        }
        // コンポジット経路がない場合だけ、OSD をデジタル映像へ直接重ねる。
        if !p.encoder.enabled {
            if osd == Some(OsdMode::Composite) {
                self.osd.burn_into_rgba(&mut self.output, opacity);
            }
            if menu {
                self.menu.burn_into_rgba(&mut self.output, 1.0);
            }
        }
        if osd == Some(OsdMode::Capture) && p.capture.enabled {
            self.osd.burn_into_rgba(&mut self.output, opacity);
        }
        if p.capture.enabled {
            self.capture.process(&self.output, &p.capture, p.seed, self.field_index, self.field.std.frame_rate());
        }
        if osd == Some(OsdMode::Direct) {
            if p.capture.enabled {
                if let Some(output) = self.capture.output_mut() {
                    self.osd.burn_into_rgba(output, opacity);
                }
            } else {
                self.osd.burn_into_rgba(&mut self.output, opacity);
            }
        }
        let t5 = Instant::now();

        self.timings =
            StageTimings { input: Duration::ZERO, camera: t0 - tc, encode: t1 - t_dv, tape: (t_dv - t0) + (t2 - t1r), rf: (t1r - t1) + (t3 - t2), decode: t4 - t3, capture: t5 - t4 };
        self.field_index += 1;
        self.output()
    }

    /// 最終フィールドのコンポジット信号 (デバッグ・波形表示用)
    pub fn composite(&self) -> &Field {
        &self.field
    }
}

/// 映像信号 (0..1) の今のフィールドのラインを出力画像へ書く。
fn write_field(src: &VideoFrame, parity: u8, out: &mut RgbaFrame) {
    let (w, h) = (out.width, out.height);
    debug_assert_eq!((src.width, src.height), (w, h));
    let parity = parity as usize;
    out.data.par_chunks_mut(w * 4).enumerate().for_each(|(row, o)| {
        if row % 2 != parity {
            return;
        }
        let sr = row.min(h - 1);
        let s = &src.data[sr * w * 3..(sr + 1) * w * 3];
        for (o, s) in o.chunks_exact_mut(4).zip(s.chunks_exact(3)) {
            for c in 0..3 {
                o[c] = (s[c] * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
            }
            o[3] = 255;
        }
    });
}
