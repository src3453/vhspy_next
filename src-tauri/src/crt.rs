//! ブラウン管シミュレーション。RetroArch の slang シェーダープリセット (.slangp) を
//! librashader + wgpu で最終出力に掛ける (crt-royale, crt-guest-advanced など)。
//!
//! GPU で描画した結果を読み戻して RGBA で返すので、プレビュー・録画・書き出しで同じ絵になる。

use librashader::presets::{ShaderFeatures, ShaderPreset, ShaderPresetPack};
use librashader::runtime::wgpu::error::FilterChainError;
use librashader::runtime::wgpu::{FilterChain, WgpuOutputView};
use librashader::runtime::{FilterChainParameters, Size, Viewport};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use vhs_core::RgbaFrame;
use vhs_core::params::CrtParams;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    name: String,
}

/// GPU はプロセスで 1 つだけ作り、プレビューと書き出しで共有する
fn gpu() -> Result<&'static Gpu, String> {
    static GPU: OnceLock<Result<Gpu, String>> = OnceLock::new();
    GPU.get_or_init(|| {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|e| format!("GPU が見つかりません: {e}"))?;
        let info = adapter.get_info();
        let mut limits = wgpu::Limits::default();
        limits.max_inter_stage_shader_variables = adapter.limits().max_inter_stage_shader_variables;
        // あれば使う (border ラップ・float テクスチャのフィルタ・sRGB/float フレームバッファ)
        let want = wgpu::Features::ADDRESS_MODE_CLAMP_TO_BORDER
            | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
            | wgpu::Features::FLOAT32_FILTERABLE;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("vhs-crt"),
            required_features: adapter.features() & want,
            required_limits: limits,
            ..Default::default()
        }))
        .map_err(|e| format!("GPU を初期化できません: {e}"))?;
        Ok(Gpu { device, queue, name: format!("{} ({:?})", info.name, info.backend) })
    })
    .as_ref()
    .map_err(|e| e.clone())
}

/// シェーダーの `#pragma parameter` (GUI のスライダー用)
#[derive(Clone, Debug, Serialize)]
pub struct ShaderParam {
    pub id: String,
    pub description: String,
    /// プリセットでの値 (上書きがなければこの値)
    pub initial: f32,
    pub min: f32,
    pub max: f32,
    pub step: f32,
}

/// 読み込んだプリセットの状態 (GUI へ渡す)
#[derive(Clone, Debug, Default, Serialize)]
pub struct CrtInfo {
    /// 読み込み済み (または読み込み中) のプリセット
    pub preset: String,
    pub loading: bool,
    pub error: Option<String>,
    pub gpu: Option<String>,
    pub params: Vec<ShaderParam>,
}

struct Loaded {
    chain: FilterChain,
    params: Vec<ShaderParam>,
    /// 最後にチェーンへ反映した上書き値
    applied: Option<BTreeMap<String, f32>>,
}

fn load(path: &str) -> Result<Loaded, String> {
    let g = gpu()?;
    let preset = ShaderPreset::try_parse(path, ShaderFeatures::NONE).map_err(|e| format!("プリセットを読めません: {e}"))?;
    let pack = ShaderPresetPack::load_from_preset::<FilterChainError>(preset)
        .map_err(|e| format!("シェーダーを読めません: {e}"))?;

    // パラメータはソース中の宣言順に並べる (ハッシュマップは順序を持たないため本文から拾い直す)
    let mut params: Vec<ShaderParam> = Vec::new();
    for pass in &pack.passes {
        let src = &pass.data;
        let mut ids: Vec<&str> = Vec::new();
        for line in src.vertex.lines().chain(src.fragment.lines()) {
            if let Some(rest) = line.trim_start().strip_prefix("#pragma parameter") {
                if let Some(id) = rest.split_whitespace().next() {
                    ids.push(id);
                }
            }
        }
        let mut rest: Vec<_> = src.parameters.keys().map(|k| k.as_str()).filter(|k| !ids.contains(k)).collect();
        rest.sort();
        ids.extend(rest);
        for id in ids {
            let Some(p) = src.parameters.get(id) else { continue };
            if params.iter().any(|q| q.id == id) {
                continue;
            }
            let initial = pack.parameters.iter().find(|m| m.name.as_str() == id).map_or(p.initial, |m| m.value);
            params.push(ShaderParam {
                id: id.to_string(),
                description: p.description.clone(),
                initial,
                min: p.minimum,
                max: p.maximum,
                step: if p.step > 0.0 { p.step } else { (p.maximum - p.minimum) / 100.0 },
            });
        }
    }

    let chain = FilterChain::load_from_pack(pack, &g.device, &g.queue, None)
        .map_err(|e| format!("シェーダーをコンパイルできません: {e}"))?;
    Ok(Loaded { chain, params, applied: None })
}

struct Target {
    w: u32,
    h: u32,
    bpr: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    buffer: wgpu::Buffer,
}

/// 1 系統分の CRT 処理 (プレビュー用・書き出し用で別々に持つ。フレーム履歴を混ぜないため)
pub struct Crt {
    /// プリセットを別スレッドで読むか (プレビューを止めないため)。書き出しでは同期で読む
    background: bool,
    loaded: Option<Loaded>,
    pending: Option<Receiver<Result<Loaded, String>>>,
    info: CrtInfo,
    /// `info` が変わるたびに増やす
    pub info_version: u64,
    input: Option<wgpu::Texture>,
    target: Option<Target>,
    frame_count: usize,
    out: Vec<u8>,
}

impl Crt {
    pub fn new(background: bool) -> Self {
        Self {
            background,
            loaded: None,
            pending: None,
            info: CrtInfo::default(),
            info_version: 0,
            input: None,
            target: None,
            frame_count: 0,
            out: Vec::new(),
        }
    }

    /// 高さ `height` のときの出力の (幅, 高さ)。4:3 の正方画素で、エンコーダのため偶数にそろえる
    pub fn output_size(height: u32) -> (u32, u32) {
        let h = height.clamp(240, 4320) & !1;
        ((h * 4 / 3 + 1) & !1, h)
    }

    /// `p` の設定で今シェーダーが掛かる状態か
    pub fn active(&self, p: &CrtParams) -> bool {
        p.enabled && !p.preset.is_empty() && self.info.preset == p.preset && self.loaded.is_some()
    }

    /// 次のフレームでプリセットを読み直す (シェーダーを編集したとき・エラーからの再試行)
    pub fn reload(&mut self) {
        self.loaded = None;
        self.pending = None;
        self.info.preset.clear();
    }

    /// 同期で読み込みを済ませる (書き出し用)。シェーダーを使うなら Ok(true)
    pub fn prepare(&mut self, p: &CrtParams) -> Result<bool, String> {
        if !p.enabled || p.preset.is_empty() {
            return Ok(false);
        }
        if self.sync(p) {
            Ok(true)
        } else {
            Err(format!("CRT シェーダー: {}", self.info.error.clone().unwrap_or_else(|| "読み込めません".into())))
        }
    }

    pub fn info(&self) -> &CrtInfo {
        &self.info
    }

    fn set_info(&mut self, f: impl FnOnce(&mut CrtInfo)) {
        f(&mut self.info);
        self.info_version += 1;
    }

    /// プリセットの読み込み状態を `p` に合わせる。使える状態なら true
    fn sync(&mut self, p: &CrtParams) -> bool {
        if !p.enabled || p.preset.is_empty() {
            return false;
        }
        if self.info.preset != p.preset {
            self.loaded = None;
            self.pending = None;
            let path = p.preset.clone();
            self.set_info(|i| *i = CrtInfo { preset: path, loading: true, ..Default::default() });
            if self.background {
                let (tx, rx) = channel();
                let path = p.preset.clone();
                std::thread::Builder::new()
                    .name("crt-load".into())
                    .spawn(move || {
                        let _ = tx.send(load(&path));
                    })
                    .unwrap();
                self.pending = Some(rx);
            } else {
                let r = load(&p.preset);
                self.finish_load(r);
            }
        }
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(r) => {
                    self.pending = None;
                    self.finish_load(r);
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.finish_load(Err("シェーダーの読み込みスレッドが異常終了しました".into()));
                }
            }
        }
        self.loaded.is_some()
    }

    fn finish_load(&mut self, r: Result<Loaded, String>) {
        let gpu_name = gpu().ok().map(|g| g.name.clone());
        match r {
            Ok(l) => {
                let params = l.params.clone();
                self.loaded = Some(l);
                self.frame_count = 0;
                self.set_info(|i| {
                    i.loading = false;
                    i.error = None;
                    i.gpu = gpu_name;
                    i.params = params;
                });
            }
            Err(e) => self.set_info(|i| {
                i.loading = false;
                i.error = Some(e);
                i.gpu = gpu_name;
            }),
        }
    }

    /// `frame` にシェーダーを掛け、高さ `height` (幅は 4:3) の RGBA を返す。
    /// CRT が無効・読み込み中・エラーのときは None (呼び出し側は元の画像を使う)
    pub fn process(&mut self, p: &CrtParams, frame: &RgbaFrame, height: u32) -> Option<(u32, u32, &[u8])> {
        if !self.sync(p) {
            return None;
        }
        match self.render(p, frame, height) {
            Ok(()) => {
                let t = self.target.as_ref().unwrap();
                Some((t.w, t.h, &self.out))
            }
            Err(e) => {
                self.loaded = None;
                self.set_info(|i| i.error = Some(e));
                None
            }
        }
    }

    fn render(&mut self, p: &CrtParams, frame: &RgbaFrame, height: u32) -> Result<(), String> {
        let g = gpu()?;
        let loaded = self.loaded.as_mut().unwrap();

        // パラメータの上書きを反映 (変わったときだけ)
        if loaded.applied.as_ref() != Some(&p.params) {
            let rt = loaded.chain.parameters();
            for sp in &loaded.params {
                rt.set_parameter_value(&sp.id, p.params.get(&sp.id).copied().unwrap_or(sp.initial));
            }
            loaded.applied = Some(p.params.clone());
        }

        // 入力テクスチャ
        let (iw, ih) = (frame.width as u32, frame.height as u32);
        if self.input.as_ref().is_none_or(|t| t.width() != iw || t.height() != ih) {
            self.input = Some(g.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("crt-input"),
                size: wgpu::Extent3d { width: iw, height: ih, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            }));
        }
        let input = self.input.as_ref().unwrap();
        g.queue.write_texture(
            input.as_image_copy(),
            &frame.data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(iw * 4), rows_per_image: None },
            input.size(),
        );

        let (ow, oh) = Self::output_size(height);
        if self.target.as_ref().is_none_or(|t| t.w != ow || t.h != oh) {
            let texture = g.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("crt-output"),
                size: wgpu::Extent3d { width: ow, height: oh, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let bpr = (ow * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let buffer = g.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("crt-readback"),
                size: (bpr * oh) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            self.target = Some(Target { w: ow, h: oh, bpr, texture, view, buffer });
        }
        let t = self.target.as_ref().unwrap();

        let mut cmd = g.device.create_command_encoder(&Default::default());
        let viewport = Viewport {
            x: 0.0,
            y: 0.0,
            mvp: None,
            size: Size::new(ow, oh),
            output: WgpuOutputView::new_from_raw(&t.view, Size::new(ow, oh), FORMAT),
        };
        loaded
            .chain
            .frame(input, &viewport, &mut cmd, self.frame_count, None)
            .map_err(|e| format!("シェーダーの描画に失敗しました: {e}"))?;
        self.frame_count += 1;
        cmd.copy_texture_to_buffer(
            t.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &t.buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(t.bpr), rows_per_image: None },
            },
            t.texture.size(),
        );
        g.queue.submit([cmd.finish()]);

        // 読み戻し (同期)
        let slice = t.buffer.slice(..);
        let (tx, rx) = channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        g.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| format!("GPU の待機に失敗しました: {e}"))?;
        rx.recv().map_err(|e| e.to_string())?.map_err(|e| format!("読み戻しに失敗しました: {e}"))?;
        {
            let data = slice.get_mapped_range().map_err(|e| format!("読み戻しに失敗しました: {e:?}"))?;
            let row = (ow * 4) as usize;
            self.out.clear();
            self.out.reserve(row * oh as usize);
            for y in 0..oh as usize {
                let s = y * t.bpr as usize;
                self.out.extend_from_slice(&data[s..s + row]);
            }
            // シェーダーによってはアルファを書かないので不透明にそろえる
            for a in self.out.iter_mut().skip(3).step_by(4) {
                *a = 255;
            }
        }
        t.buffer.unmap();
        Ok(())
    }
}
