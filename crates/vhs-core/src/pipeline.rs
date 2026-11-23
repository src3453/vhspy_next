//! 全体のパイプライン: RGB → コンポジット → テープ → 受像機 → RGBA。

use crate::camera::{CameraInput, CameraSim};
use crate::capture::{CaptureSim, CodecStats};
use crate::decoder::Decoder;
use crate::encoder::encode_field;
use crate::frame::{Field, LinearFrame, RgbFrame, RgbaFrame, VideoFrame};
use crate::osd::Osd;
use crate::params::{OsdMode, Params};
use crate::rf::RfSim;
use crate::tape::TapeSim;
use crate::timing::Standard;
use std::time::{Duration, Instant};

/// 放送受信側の RF の乱数系列をデッキ→テレビ側と分けるための値
const RF_BROADCAST_SEED: u64 = 0x5246_4252_4f41_4443;

#[derive(Clone, Copy, Debug, Default)]
pub struct StageTimings {
    pub camera: Duration,
    pub encode: Duration,
    pub tape: Duration,
    pub rf: Duration,
    pub decode: Duration,
    pub capture: Duration,
}

impl StageTimings {
    pub fn total(&self) -> Duration {
        self.camera + self.encode + self.tape + self.rf + self.decode + self.capture
    }
}

pub struct Pipeline {
    params: Params,
    camera: CameraSim,
    field: Field,
    tape: TapeSim,
    /// 放送受信 (テープの前) の RF
    rf_broadcast: RfSim,
    /// デッキ → テレビ (テープの後) の RF
    rf: RfSim,
    decoder: Decoder,
    capture: CaptureSim,
    osd: Osd,
    /// エンコーダに渡す映像信号の作業領域 (カメラなしの入力、OSD の焼き込み)
    video: VideoFrame,
    output: RgbaFrame,
    field_index: u64,
    pub timings: StageTimings,
}

impl Pipeline {
    pub fn new(params: Params) -> Self {
        let std = params.standard.spec();
        let (w, h) = std.image_size();
        let mut camera = CameraSim::default();
        camera.set_field_rate(std.field_rate);
        Self {
            params,
            camera,
            field: Field::new(std),
            tape: TapeSim::new(std),
            rf_broadcast: RfSim::default(),
            rf: RfSim::default(),
            decoder: Decoder::new(std),
            capture: CaptureSim::default(),
            osd: Osd::new(std),
            video: VideoFrame::new(w, h),
            output: RgbaFrame::new(w, h),
            field_index: 0,
            timings: StageTimings::default(),
        }
    }

    pub fn params(&self) -> &Params {
        &self.params
    }

    /// パラメータを差し替える。テレビ方式が変わったときは内部状態を作り直す
    /// (画像サイズが変わるので、入力も [`Pipeline::input_size`] に合わせ直すこと)。
    pub fn set_params(&mut self, params: Params) {
        if params.standard != self.params.standard {
            let index = self.field_index;
            *self = Self::new(params);
            self.field_index = index;
        } else {
            self.params = params;
        }
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
        self.run(CameraInput::Rgb(input))
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
        self.run(CameraInput::Linear(input))
    }

    fn run(&mut self, src: CameraInput) -> &RgbaFrame {
        self.field.set_index(self.field_index);
        let p = &self.params;
        let osd = (p.osd.enabled && self.osd.update(&p.osd, self.field_index)).then_some(p.osd.mode);
        let opacity = p.osd.opacity.clamp(0.0, 1.0);

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
        let t0 = Instant::now();
        encode_field(input, &mut self.field, &p.encoder);
        let t1 = Instant::now();
        // 放送受信: 受信時の劣化ごとテープに記録される (デッキ→テレビの RF とは独立した雑音)
        if p.rf.enabled && p.rf.placement.before_tape() {
            self.rf_broadcast.process(&mut self.field, &p.rf, p.seed ^ RF_BROADCAST_SEED);
        }
        let t1r = Instant::now();
        if p.tape.enabled {
            self.tape.process(&mut self.field, &p.tape, &p.transport, p.seed);
        }
        if osd == Some(OsdMode::Composite) {
            self.osd.insert_composite(&mut self.field, opacity);
        }
        let t2 = Instant::now();
        if p.rf.enabled && p.rf.placement.after_tape() {
            self.rf.process(&mut self.field, &p.rf, p.seed);
        }
        let t3 = Instant::now();
        self.decoder.decode_field(&self.field, &p.decoder, p.output.deinterlace, &mut self.output);
        let t4 = Instant::now();
        if p.capture.enabled {
            self.capture.process(&self.output, &p.capture, p.seed, self.field_index, self.field.std.frame_rate());
        }
        let t5 = Instant::now();

        self.timings =
            StageTimings { camera: t0 - tc, encode: t1 - t0, tape: t2 - t1r, rf: (t1r - t1) + (t3 - t2), decode: t4 - t3, capture: t5 - t4 };
        self.field_index += 1;
        self.output()
    }

    /// 最終フィールドのコンポジット信号 (デバッグ・波形表示用)
    pub fn composite(&self) -> &Field {
        &self.field
    }
}
