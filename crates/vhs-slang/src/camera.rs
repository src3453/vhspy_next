//! 撮像管カメラ (`shaders/camera.slang`) の wgpu 参照ホスト。
//!
//! 入力 (RGBA float。シーンリニアな HDR か表示用の値) を受け、有効映像の大きさのカメラ出力を
//! `output_texture()` に書く。この出力は [`crate::GpuComposite::set_input_texture`] にそのまま渡せる。
//!
//! 管ごとの個体差と感度・暗電流マップは CPU 版と同じもの ([`vhs_core::camera::TubeMaps`]) をアップロードする。
//! 管に依らない係数も CPU 版 (`vhs_core::camera`) と同じ式でここで計算して、シェーダーに渡す。

use std::sync::mpsc::channel;

use vhs_core::camera::{Tube, TubeMaps};
use vhs_core::params::CameraParams;
use vhs_core::{Standard, TvStandard};

use crate::GpuContext;

pub const CAMERA_WGSL: &str = include_str!("../generated/camera.wgsl");

/// 入力 (または光学系のパラメータ) を差し替えたときに流すカーネル
pub const CAMERA_OPTICS_KERNELS: [&str; 5] = ["camOptics", "camRowSum", "camMean", "camBlurH", "camBlurV"];
/// フィールドごとに流すカーネル
pub const CAMERA_FIELD_KERNELS: [&str; 2] = ["camTarget", "camProcess"];

/// 真空管プリアンプの動作点 (vhs-core の camera::AMP_BIAS と同じ)
const AMP_BIAS: f32 = 0.35;

/// `CameraGpuParams` (shaders/camera.slang) と同じ並び。144 バイト
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CameraGpuParams {
    pub width: u32,
    pub height: u32,
    pub src_width: u32,
    pub src_height: u32,
    pub input_linear: u32,
    pub parity: u32,
    pub prime: u32,
    pub field_index: u32,
    pub seed_lo: u32,
    pub seed_hi: u32,
    pub act: u32,
    pub pad0: u32,
    pub gain: f32,
    pub crosstalk: f32,
    pub flare: f32,
    pub sigma: f32,
    pub k_decay: f32,
    pub k_rise: f32,
    pub act_level: f32,
    pub bias: f32,
    pub burn_recovery: f32,
    pub hysteresis: f32,
    pub h_relax: f32,
    pub amp_cp: f32,
    pub amp_cn: f32,
    pub amp_f0: f32,
    pub amp_scale: f32,
    pub proc_gain: f32,
    pub noise: f32,
    pub aperture: f32,
    pub knee: f32,
    pub knee_slope: f32,
    pub white_clip: f32,
    pub camera_gamma: f32,
    pub pad1: f32,
    pub pad2: f32,
}

const _: () = assert!(std::mem::size_of::<CameraGpuParams>() == 144);

/// `TubeGpu` (shaders/camera.slang) と同じ並び。48 バイト
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TubeGpu {
    pub gamma: f32,
    pub dx: f32,
    pub dy: f32,
    pub scale: f32,
    pub photo_lag: f32,
    pub cap_lag: f32,
    pub beam: f32,
    pub qsat: f32,
    pub black: f32,
    pub burn_rate: f32,
    pub pad0: f32,
    pub pad1: f32,
}

const _: () = assert!(std::mem::size_of::<TubeGpu>() == 48);

/// `CamPixel` の大きさ (float4 × 5)
const STATE_BYTES: u64 = 80;

impl CameraGpuParams {
    /// フィールドに依らない係数を CPU 版 (camera.rs の optics / target / process_amp) と同じ式で求める
    pub fn new(p: &CameraParams, seed: u64, field_rate: f32) -> Self {
        let a = p.aging.clamp(0.0, 1.0);
        let tau = p.photo_lag_fields.max(0.1);
        let drive = p.amp_drive.max(0.0);
        let asym = p.amp_asymmetry.clamp(-1.0, 1.0);
        let (cp, cn) = (drive * (1.0 + asym), drive * (1.0 - asym));
        let raw = |s: f32| {
            let u = s - AMP_BIAS;
            let c = if u > 0.0 { cp } else { cn };
            if c < 1e-4 { u } else { (c * u).tanh() / c }
        };
        let (f0, f1) = (raw(0.0), raw(1.0));
        Self {
            seed_lo: seed as u32,
            seed_hi: (seed >> 32) as u32,
            act: u32::from(p.act),
            gain: 2f32.powf(p.exposure_ev),
            crosstalk: p.crosstalk.clamp(0.0, 0.33),
            flare: p.flare.max(0.0),
            sigma: p.resolution_px.max(0.0) + 0.8 * a,
            k_decay: 1.0 - (-1.0 / tau).exp(),
            k_rise: 1.0 - (-2.0 / tau).exp(),
            act_level: p.act_level.max(0.1) * 2.0,
            bias: p.bias_light.max(0.0),
            burn_recovery: p.burn_recovery.max(0.0) / field_rate,
            hysteresis: p.hysteresis.max(0.0),
            h_relax: 1.0 - (-1.0 / p.hysteresis_fields.max(0.1)).exp(),
            amp_cp: cp,
            amp_cn: cn,
            amp_f0: f0,
            amp_scale: if (f1 - f0).abs() > 1e-6 { 1.0 / (f1 - f0) } else { 1.0 },
            proc_gain: 1.0 / (1.0 - 0.4 * a),
            noise: p.noise_pct.max(0.0) * 0.01 * (1.0 + 2.0 * a),
            aperture: p.aperture,
            knee: p.knee,
            knee_slope: p.knee_slope.max(0.0),
            white_clip: p.white_clip,
            camera_gamma: p.camera_gamma.max(0.05),
            ..Default::default()
        }
    }
}

impl TubeGpu {
    /// 管ごとに決まる係数 (camera.rs の target と同じ式)
    pub fn new(p: &CameraParams, t: &Tube, field_rate: f32) -> Self {
        let a = p.aging.clamp(0.0, 1.0);
        let beam = (p.beam_current.max(0.05) * t.beam * 2.0).max(0.05);
        Self {
            gamma: t.gamma,
            dx: t.dx,
            dy: t.dy,
            scale: t.scale,
            photo_lag: (p.photo_lag.max(0.0) * t.photo_lag + 0.3 * a).min(0.95),
            cap_lag: p.capacitive_lag.max(0.0) * t.cap_lag,
            beam,
            qsat: (p.target_saturation.max(p.beam_current) * 2.0).max(beam),
            black: p.dark_current.max(0.0) * (1.0 + 5.0 * a) + p.bias_light.max(0.0) - t.black,
            burn_rate: p.burn_rate.max(0.0) * t.burn / field_rate,
            ..Default::default()
        }
    }
}

/// 感度・暗電流マップを作り直すかどうかの判定に使う値 (vhs-core の camera::MapKey と同じ項目)
#[derive(Clone, PartialEq)]
struct MapKey {
    seed: u64,
    vals: [u32; 6],
    blemishes: u32,
}

impl MapKey {
    fn new(p: &CameraParams, seed: u64) -> Self {
        let v = [p.shading, p.aging, p.mismatch, p.dark_current, p.target_gamma, p.registration_px];
        Self { seed, vals: v.map(f32::to_bits), blemishes: p.blemishes }
    }
}

/// 1 台のカメラの GPU 側の状態 (ターゲットの電荷・残像・焼き付きなど、フィールドをまたぐ値を持つ)
pub struct GpuCamera {
    std: &'static Standard,
    w: u32,
    h: u32,
    params: CameraParams,
    seed: u64,
    optics: Vec<(&'static str, wgpu::ComputePipeline)>,
    field: Vec<(&'static str, wgpu::ComputePipeline)>,
    layout: wgpu::BindGroupLayout,
    param_buffer: wgpu::Buffer,
    tube_buffer: wgpu::Buffer,
    /// maps, lin, tmp, flux, state, rows (バインディング 4..=9)
    buffers: [wgpu::Buffer; 6],
    output: wgpu::Texture,
    input: Option<wgpu::Texture>,
    input_linear: bool,
    bind_group: Option<wgpu::BindGroup>,
    /// 入力か光学系のパラメータが変わり、まだ光学系のカーネルを流していない
    optics_dirty: bool,
    tubes: Option<[Tube; 3]>,
    key: Option<MapKey>,
    last_field: Option<u64>,
    readback: wgpu::Buffer,
    readback_row: u32,
}

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

impl GpuCamera {
    pub fn new(gpu: &GpuContext, standard: TvStandard, params: &CameraParams, seed: u64) -> Self {
        let device = &gpu.device;
        let std = standard.spec();
        let (w, h) = std.image_size();
        let (w, h) = (w as u32, h as u32);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vhs-camera"),
            source: wgpu::ShaderSource::Wgsl(CAMERA_WGSL.into()),
        });

        // バインディング番号は slangc が宣言順に振ったもの (generated/camera.wgsl を参照)
        let entries = vec![
            storage_entry(0, true),
            storage_entry(1, true),
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::Rgba32Float,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            },
            storage_entry(4, true),
            storage_entry(5, false),
            storage_entry(6, false),
            storage_entry(7, false),
            storage_entry(8, false),
            storage_entry(9, false),
        ];
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vhs-camera"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vhs-camera"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make = |name: &'static str| {
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(name),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(name),
                compilation_options: Default::default(),
                cache: None,
            });
            (name, pipeline)
        };
        let optics = CAMERA_OPTICS_KERNELS.iter().map(|n| make(n)).collect();
        let field = CAMERA_FIELD_KERNELS.iter().map(|n| make(n)).collect();

        let buffer = |label: &str, size: u64| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let n = (w * h) as u64;
        let param_buffer = buffer("camera-params", std::mem::size_of::<CameraGpuParams>() as u64);
        let tube_buffer = buffer("camera-tubes", 3 * std::mem::size_of::<TubeGpu>() as u64);
        let buffers = [
            buffer("camMaps", 32 * n),
            buffer("camLin", 16 * n),
            buffer("camTmp", 16 * n),
            buffer("camFlux", 16 * n),
            buffer("camState", STATE_BYTES * n),
            buffer("camRows", 16 * (h as u64 + 1)),
        ];
        let output = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vhs-camera-output"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback_row = (w * 16).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vhs-camera-readback"),
            size: (readback_row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Self {
            std,
            w,
            h,
            params: params.clone(),
            seed,
            optics,
            field,
            layout,
            param_buffer,
            tube_buffer,
            buffers,
            output,
            input: None,
            input_linear: true,
            bind_group: None,
            optics_dirty: true,
            tubes: None,
            key: None,
            last_field: None,
            readback,
            readback_row,
        }
    }

    /// カメラの出力 (有効映像の幅 × フレームのライン数、RGBA32F、表示用の値)
    pub fn output_texture(&self) -> &wgpu::Texture {
        &self.output
    }

    pub fn output_size(&self) -> (usize, usize) {
        (self.w as usize, self.h as usize)
    }

    /// パラメータを差し替える。光学系は次のフィールドで流し直す
    pub fn set_params(&mut self, params: &CameraParams, seed: u64) {
        if &self.params != params || self.seed != seed {
            self.params = params.clone();
            self.seed = seed;
            self.optics_dirty = true;
        }
    }

    /// 入力 (RGBA float、幅 × 高さ × 4) を送る。`linear` なら値をシーンリニアな光 (白 = 1、上限なし) として読み、
    /// そうでなければ表示用の値 (0..1) として 2.2 乗で線形化する
    pub fn upload_input(&mut self, gpu: &GpuContext, rgba: &[f32], width: u32, height: u32, linear: bool) {
        assert_eq!(rgba.len(), (width * height * 4) as usize);
        if self.input.as_ref().is_none_or(|t| t.width() != width || t.height() != height) {
            self.input = Some(gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("vhs-camera-input"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            }));
            self.bind_group = None;
        }
        let input = self.input.as_ref().unwrap();
        gpu.queue.write_texture(
            input.as_image_copy(),
            bytemuck::cast_slice(rgba),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 16), rows_per_image: None },
            input.size(),
        );
        self.input_linear = linear;
        self.optics_dirty = true;
    }

    /// 管の個体差と感度・暗電流マップを (変わっていれば) 作り直して送る
    fn update_maps(&mut self, gpu: &GpuContext) {
        let key = MapKey::new(&self.params, self.seed);
        if self.key.as_ref() == Some(&key) && self.tubes.is_some() {
            return;
        }
        let maps = TubeMaps::new(&self.params, self.seed, self.w as usize, self.h as usize);
        // 画素ごとに [感度 RGB_, 暗電流 RGB_] と交互に並べる
        let (s, d) = (&maps.sens, &maps.dark);
        let packed: Vec<f32> =
            (0..s[0].len()).flat_map(|i| [s[0][i], s[1][i], s[2][i], 0.0, d[0][i], d[1][i], d[2][i], 0.0]).collect();
        gpu.queue.write_buffer(&self.buffers[0], 0, bytemuck::cast_slice(&packed));
        self.tubes = Some(maps.tubes);
        self.key = Some(key);
    }

    fn bind_group(&mut self, gpu: &GpuContext) -> &wgpu::BindGroup {
        if self.bind_group.is_none() {
            let input = self.input.as_ref().expect("input uploaded");
            let input_view = input.create_view(&Default::default());
            let output_view = self.output.create_view(&Default::default());
            let mut entries = vec![
                wgpu::BindGroupEntry { binding: 0, resource: self.param_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: self.tube_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&input_view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&output_view) },
            ];
            for (i, b) in self.buffers.iter().enumerate() {
                entries.push(wgpu::BindGroupEntry { binding: 4 + i as u32, resource: b.as_entire_binding() });
            }
            self.bind_group = Some(gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("vhs-camera"),
                layout: &self.layout,
                entries: &entries,
            }));
        }
        self.bind_group.as_ref().unwrap()
    }

    fn dispatch_size(&self, kernel: &str) -> (u32, u32, u32) {
        match kernel {
            "camRowSum" => (self.h.div_ceil(64), 1, 1),
            "camMean" => (1, 1, 1),
            "camProcess" => (self.w.div_ceil(8), (self.h / 2).div_ceil(8), 1),
            _ => (self.w.div_ceil(8), self.h.div_ceil(8), 1),
        }
    }

    /// 1 フィールドぶん処理する (`vhs_core::camera::CameraSim::process` に相当)。
    /// 出力テクスチャのうち、このフィールドの走査線 (`field_index` の偶奇) が更新される
    pub fn process_field(&mut self, gpu: &GpuContext, field_index: u64) {
        self.update_maps(gpu);
        let field_rate = self.std.field_rate;
        let tubes = self.tubes.unwrap();
        let tube_gpu: [TubeGpu; 3] = std::array::from_fn(|c| TubeGpu::new(&self.params, &tubes[c], field_rate));
        gpu.queue.write_buffer(&self.tube_buffer, 0, bytemuck::cast_slice(&tube_gpu));

        // フィールドが連続していなければ (初回・無効化からの復帰) 定常状態から始める
        let prime = self.last_field.is_none_or(|f| f + 1 != field_index);
        self.last_field = Some(field_index);
        let input = self.input.as_ref().expect("input uploaded");
        let mut p = CameraGpuParams::new(&self.params, self.seed, field_rate);
        p.width = self.w;
        p.height = self.h;
        p.src_width = input.width();
        p.src_height = input.height();
        p.input_linear = u32::from(self.input_linear);
        p.parity = (field_index & 1) as u32;
        p.prime = u32::from(prime);
        p.field_index = field_index as u32;
        gpu.queue.write_buffer(&self.param_buffer, 0, bytemuck::bytes_of(&p));

        let run_optics = std::mem::take(&mut self.optics_dirty);
        let dispatches: Vec<(wgpu::ComputePipeline, (u32, u32, u32))> = self
            .optics
            .iter()
            .filter(|_| run_optics)
            .chain(&self.field)
            .map(|(name, pipeline)| (pipeline.clone(), self.dispatch_size(name)))
            .collect();
        let group = self.bind_group(gpu).clone();
        let mut cmd = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = cmd.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("vhs-camera"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, &group, &[]);
            for (pipeline, (x, y, z)) in &dispatches {
                pass.set_pipeline(pipeline);
                pass.dispatch_workgroups(*x, *y, *z);
            }
        }
        gpu.queue.submit([cmd.finish()]);
    }

    /// 出力テクスチャを読み戻す (RGBA float、幅 × 高さ × 4)
    pub fn read_output(&self, gpu: &GpuContext) -> Vec<f32> {
        let (w, h) = (self.w as usize, self.h as usize);
        let mut cmd = gpu.device.create_command_encoder(&Default::default());
        cmd.copy_texture_to_buffer(
            self.output.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.readback_row),
                    rows_per_image: None,
                },
            },
            self.output.size(),
        );
        gpu.queue.submit([cmd.finish()]);
        let slice = self.readback.slice(..);
        let (tx, rx) = channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU wait failed");
        rx.recv().expect("map callback dropped").expect("readback map failed");
        let mut out = Vec::with_capacity(w * h * 4);
        {
            let data = slice.get_mapped_range().expect("mapped range");
            for y in 0..h {
                let start = y * self.readback_row as usize;
                out.extend_from_slice(bytemuck::cast_slice(&data[start..start + w * 16]));
            }
        }
        self.readback.unmap();
        out
    }
}
