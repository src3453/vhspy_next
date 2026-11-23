//! 全体のパイプライン: RGB → コンポジット → テープ → 受像機 → RGBA。

use crate::camera::CameraSim;
use crate::capture::{CaptureSim, CodecStats};
use crate::decoder::Decoder;
use crate::encoder::encode_field;
use crate::frame::{Field, RgbFrame, RgbaFrame};
use crate::osd::Osd;
use crate::params::{OsdMode, Params};
use crate::rf::RfSim;
use crate::tape::TapeSim;
use crate::timing::{ACTIVE_WIDTH, FRAME_LINES};
use std::time::{Duration, Instant};

/// 入力画像はこの解像度に合わせると内部リサイズが不要になる
pub const INPUT_WIDTH: usize = ACTIVE_WIDTH;
pub const INPUT_HEIGHT: usize = FRAME_LINES;
/// 出力画像の解像度 (表示時に 4:3 へ引き伸ばす。ピクセルアスペクト比 ≒ 0.849)
pub const OUTPUT_WIDTH: usize = ACTIVE_WIDTH;
pub const OUTPUT_HEIGHT: usize = FRAME_LINES;

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
    rf: RfSim,
    decoder: Decoder,
    capture: CaptureSim,
    osd: Osd,
    /// OSD を焼き込んだ入力画像の作業領域
    burned: Option<RgbFrame>,
    output: RgbaFrame,
    field_index: u64,
    pub timings: StageTimings,
}

impl Pipeline {
    pub fn new(params: Params) -> Self {
        Self {
            params,
            camera: CameraSim::default(),
            field: Field::default(),
            tape: TapeSim::default(),
            rf: RfSim::default(),
            decoder: Decoder::default(),
            capture: CaptureSim::default(),
            osd: Osd::default(),
            burned: None,
            output: RgbaFrame::new(OUTPUT_WIDTH, OUTPUT_HEIGHT),
            field_index: 0,
            timings: StageTimings::default(),
        }
    }

    pub fn params(&self) -> &Params {
        &self.params
    }

    pub fn set_params(&mut self, params: Params) {
        self.params = params;
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

    /// 1 フィールドぶん処理する。入力 1 枚 = 1 フィールド (59.94 Hz 想定)。
    pub fn process_field(&mut self, input: &RgbFrame) -> &RgbaFrame {
        let resized;
        let input = if input.width == INPUT_WIDTH && input.height == INPUT_HEIGHT {
            input
        } else {
            resized = input.resized(INPUT_WIDTH, INPUT_HEIGHT);
            &resized
        };

        self.field.set_index(self.field_index);
        let p = &self.params;
        let osd = (p.osd.enabled && self.osd.update(&p.osd, self.field_index)).then_some(p.osd.mode);
        let opacity = p.osd.opacity.clamp(0.0, 1.0);

        let tc = Instant::now();
        // 撮像管カメラ (OSD の日付などはカメラの後段で焼き込まれる)
        let input = if p.camera.enabled {
            self.camera.process(input, &p.camera, p.seed, self.field_index)
        } else {
            input
        };
        let t0 = Instant::now();
        let input = if osd == Some(OsdMode::Tape) {
            let buf = self.burned.get_or_insert_with(|| RgbFrame::new(INPUT_WIDTH, INPUT_HEIGHT));
            buf.data.copy_from_slice(&input.data);
            self.osd.burn_into(buf, opacity);
            &*buf
        } else {
            input
        };
        encode_field(input, &mut self.field, &p.encoder);
        let t1 = Instant::now();
        if p.tape.enabled {
            self.tape.process(&mut self.field, &p.tape, &p.transport, p.seed);
        }
        if osd == Some(OsdMode::Composite) {
            self.osd.insert_composite(&mut self.field, opacity);
        }
        let t2 = Instant::now();
        if p.rf.enabled {
            self.rf.process(&mut self.field, &p.rf, p.seed);
        }
        let t3 = Instant::now();
        self.decoder.decode_field(&self.field, &p.decoder, p.output.deinterlace, &mut self.output);
        let t4 = Instant::now();
        if p.capture.enabled {
            self.capture.process(&self.output, &p.capture, p.seed, self.field_index);
        }
        let t5 = Instant::now();

        self.timings =
            StageTimings { camera: t0 - tc, encode: t1 - t0, tape: t2 - t1, rf: t3 - t2, decode: t4 - t3, capture: t5 - t4 };
        self.field_index += 1;
        self.output()
    }

    /// 最終フィールドのコンポジット信号 (デバッグ・波形表示用)
    pub fn composite(&self) -> &Field {
        &self.field
    }
}
