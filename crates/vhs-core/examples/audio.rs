//! 音声の劣化処理だけをかけるフィルタ。標準入力の f32le / 48 kHz / ステレオを処理して標準出力へ流す。
//!
//! ffmpeg -i in.mp4 -vn -ac 2 -ar 48000 -f f32le - |
//!   cargo run -p vhs-core --release --example audio -- [params.json] |
//!   ffmpeg -f f32le -ar 48000 -ac 2 -i - out.wav

use std::io::{Read, Write};
use vhs_core::Params;
use vhs_core::audio::AudioProcessor;

fn main() {
    let params: Params = match std::env::args().nth(1) {
        Some(p) => serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap(),
        None => Params::default(),
    };
    let mut proc = AudioProcessor::new(&params, 48000);
    let mut skip = proc.latency() * 2;
    let (mut stdin, mut stdout) = (std::io::stdin().lock(), std::io::stdout().lock());
    let mut bytes = vec![0u8; 4096 * 8];
    let mut buf = Vec::new();
    loop {
        let mut filled = 0;
        while filled < bytes.len() {
            match stdin.read(&mut bytes[filled..]).unwrap() {
                0 => break,
                n => filled += n,
            }
        }
        let frames = filled / 8;
        buf.clear();
        buf.extend(bytes[..frames * 8].chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())));
        if frames == 0 {
            // 遅延線の残りを押し出す
            buf.resize(proc.latency() * 2, 0.0);
        }
        proc.process(&mut buf);
        let n = skip.min(buf.len());
        skip -= n;
        for v in &buf[n..] {
            stdout.write_all(&v.to_le_bytes()).unwrap();
        }
        if frames == 0 || filled < bytes.len() {
            if frames != 0 {
                let mut tail = vec![0f32; proc.latency() * 2];
                proc.process(&mut tail);
                for v in &tail {
                    stdout.write_all(&v.to_le_bytes()).unwrap();
                }
            }
            break;
        }
    }
}
