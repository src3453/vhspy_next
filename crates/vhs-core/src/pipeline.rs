//! 全体のパイプライン: RGB → コンポジット → テープ → 受像機 → RGBA。

use crate::decoder::Decoder;
use crate::encoder::encode_field;
use crate::frame::{Field, RgbFrame, RgbaFrame};
use crate::params::Params;
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
    pub encode: Duration,
    pub tape: Duration,
    pub decode: Duration,
}

impl StageTimings {
    pub fn total(&self) -> Duration {
        self.encode + self.tape + self.decode
    }
}

pub struct Pipeline {
    params: Params,
    field: Field,
    tape: TapeSim,
    decoder: Decoder,
    output: RgbaFrame,
    field_index: u64,
    pub timings: StageTimings,
}

impl Pipeline {
    pub fn new(params: Params) -> Self {
        Self {
            params,
            field: Field::default(),
            tape: TapeSim::default(),
            decoder: Decoder::default(),
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

    pub fn output(&self) -> &RgbaFrame {
        &self.output
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

        let t0 = Instant::now();
        encode_field(input, &mut self.field, &p.encoder);
        let t1 = Instant::now();
        if p.tape.enabled {
            self.tape.process(&mut self.field, &p.tape, &p.transport, p.seed);
        }
        let t2 = Instant::now();
        self.decoder.decode_field(&self.field, &p.decoder, p.output.deinterlace, &mut self.output);
        let t3 = Instant::now();

        self.timings = StageTimings { encode: t1 - t0, tape: t2 - t1, decode: t3 - t2 };
        self.field_index += 1;
        &self.output
    }

    /// 最終フィールドのコンポジット信号 (デバッグ・波形表示用)
    pub fn composite(&self) -> &Field {
        &self.field
    }
}
