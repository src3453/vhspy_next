//! Slang で書いたコンポジット信号のエンコード → デコード (PoC) を wgpu で動かす参照ホスト。
//!
//! シェーダーの本体は `shaders/*.slang`。`generated/composite.wgsl` は `scripts/build-shaders.ps1`
//! で slangc から生成したもので、slangc なしでもビルドできるようにリポジトリに入れている。
//! Unity / Unreal 向けの HLSL も同じスクリプトで同じソースから出す。
//!
//! このホストはエンジン側の実装の手本と、CPU 版 (`vhs-core`) との比較テストに使う。

use std::sync::mpsc::channel;

pub mod camera;
pub use camera::{CameraGpuParams, GpuCamera, TubeGpu};

use vhs_core::params::{DecoderParams, EncoderParams, YcSeparation};
use vhs_core::{Standard, TvStandard};

pub const WGSL: &str = include_str!("../generated/composite.wgsl");

/// 入力を差し替えたときに 1 回だけ流すカーネル (両フィールドの行をまとめて作る)
pub const PREPARE_KERNEL: &str = "vhsPrepare";

/// フィールドごとにディスパッチ順に流すカーネル
pub const KERNELS: [&str; 5] = ["vhsEncode", "vhsSyncAfc", "vhsBurstLock", "vhsSeparateDemod", "vhsResolve"];

/// shaders/composite.slang の同名の定数と同じ値にすること
pub const ENCODE_CHUNK: u32 = 64;
pub const DEMOD_CHUNK: u32 = 64;
pub const SERIAL_THREADS: u32 = 320;
pub const PREPARE_TILE_W: u32 = 32;
pub const PREPARE_TILE_R: u32 = 4;

/// カーネルごとのワークグループ数
pub fn dispatch_size(kernel: &str, s: &Standard) -> (u32, u32, u32) {
    let lines = s.field_lines as u32;
    match kernel {
        "vhsEncode" => ((lines * (s.line_samples as u32).div_ceil(ENCODE_CHUNK)).div_ceil(64), 1, 1),
        "vhsSyncAfc" | "vhsBurstLock" => (1, 1, 1),
        "vhsSeparateDemod" => ((lines * (s.active_width as u32).div_ceil(DEMOD_CHUNK)).div_ceil(64), 1, 1),
        "vhsPrepare" => ((s.active_width as u32).div_ceil(PREPARE_TILE_W), (2 * lines).div_ceil(PREPARE_TILE_R), 1),
        "vhsResolve" => ((s.active_width as u32).div_ceil(8), lines.div_ceil(8), 1),
        _ => unreachable!("unknown kernel {kernel}"),
    }
}

/// `CompositeParams` (shaders/composite.slang) と同じ並び。4 バイトのスカラーだけ、100 バイト
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CompositeParams {
    pub standard: u32,
    pub parity: u32,
    pub frame_no: u32,
    pub field_index: u32,
    pub src_width: u32,
    pub src_height: u32,
    pub input_linear: u32,
    pub yc_mode: u32,
    pub luma_bw_mhz: f32,
    pub i_bw_mhz: f32,
    pub q_bw_mhz: f32,
    pub uv_bw_mhz: f32,
    pub chroma_gain: f32,
    pub chroma_bw_mhz: f32,
    pub h_pll: f32,
    pub burst_pll: f32,
    pub contrast: f32,
    pub brightness: f32,
    pub saturation: f32,
    pub hue_deg: f32,
    pub acc: u32,
    pub color_killer: u32,
    pub pal_delay_line: u32,
    pub output_linear: u32,
    pub vcxo_offset_hz: f32,
}

const _: () = assert!(std::mem::size_of::<CompositeParams>() == 100);

impl CompositeParams {
    /// GPU 版が扱う範囲のパラメータを写す。3 次元 Y/C 分離は未対応なのでラインくし形にする
    pub fn new(standard: TvStandard, enc: &EncoderParams, dec: &DecoderParams) -> Self {
        Self {
            standard: u32::from(standard == TvStandard::Pal),
            parity: 0,
            frame_no: 0,
            field_index: 0,
            src_width: 0,
            src_height: 0,
            input_linear: 0,
            yc_mode: u32::from(dec.yc_separation != YcSeparation::Notch),
            luma_bw_mhz: enc.luma_bw_mhz,
            i_bw_mhz: enc.i_bw_mhz,
            q_bw_mhz: enc.q_bw_mhz,
            uv_bw_mhz: enc.uv_bw_mhz,
            chroma_gain: enc.chroma_gain,
            chroma_bw_mhz: dec.chroma_bw_mhz,
            h_pll: dec.h_pll,
            burst_pll: dec.burst_pll,
            contrast: dec.contrast,
            brightness: dec.brightness,
            saturation: dec.saturation,
            hue_deg: dec.hue_deg,
            acc: u32::from(dec.acc),
            color_killer: u32::from(dec.color_killer),
            pal_delay_line: u32::from(dec.pal_delay_line),
            output_linear: 0,
            vcxo_offset_hz: dec.vcxo_offset_hz,
        }
    }
}

pub struct GpuContext {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter_name: String,
    /// カーネルごとの時間を測れる (TIMESTAMP_QUERY が使える)
    pub timestamps: bool,
}

impl GpuContext {
    pub fn new() -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|e| format!("no GPU adapter: {e}"))?;
        let info = adapter.get_info();
        let features = adapter.features() & wgpu::Features::TIMESTAMP_QUERY;
        // フィールド単位の逐次カーネルは SERIAL_THREADS (= 320) スレッドのワークグループを使う
        let adapter_limits = adapter.limits();
        if adapter_limits.max_compute_invocations_per_workgroup < SERIAL_THREADS {
            return Err(format!(
                "adapter allows only {} invocations per workgroup",
                adapter_limits.max_compute_invocations_per_workgroup
            ));
        }
        let limits = wgpu::Limits {
            max_compute_invocations_per_workgroup: SERIAL_THREADS,
            max_compute_workgroup_size_x: SERIAL_THREADS,
            ..Default::default()
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("vhs-slang"),
            required_features: features,
            required_limits: limits,
            ..Default::default()
        }))
        .map_err(|e| format!("device request failed: {e}"))?;
        Ok(Self {
            device,
            queue,
            adapter_name: format!("{} ({:?})", info.name, info.backend),
            timestamps: !features.is_empty(),
        })
    }
}

/// 1 回の submit にまとめるフィールド数の上限 (フィールドごとのパラメータを別々の場所に置く)
const PARAM_SLOTS: usize = 64;
/// ストレージバッファのオフセットの揃え (WebGPU の既定の上限)
const PARAM_STRIDE: u64 = 256;

/// 1 つの映像系列の GPU 側の状態 (AFC の位置など、フィールドをまたぐ値を持つ)
pub struct GpuComposite {
    std: &'static Standard,
    params: CompositeParams,
    pipelines: Vec<wgpu::ComputePipeline>,
    prepare: wgpu::ComputePipeline,
    /// 送った入力をまだ vhsPrepare に通していない
    input_dirty: bool,
    layout: wgpu::BindGroupLayout,
    param_buffer: wgpu::Buffer,
    /// video, signal, luma, detected, lines, state (バインディング 3..=8)
    buffers: [wgpu::Buffer; 6],
    output: wgpu::Texture,
    input: Option<wgpu::Texture>,
    readback: wgpu::Buffer,
    readback_row: u32,
    field_index: u64,
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

impl GpuComposite {
    pub fn new(gpu: &GpuContext, standard: TvStandard, params: CompositeParams) -> Self {
        let device = &gpu.device;
        let std = standard.spec();
        let (aw, ah) = std.image_size();
        let (aw, ah) = (aw as u32, ah as u32);

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vhs-composite"),
            source: wgpu::ShaderSource::Wgsl(WGSL.into()),
        });

        // バインディング番号は slangc が宣言順に振ったもの (generated/composite.wgsl を参照)
        let mut entries = vec![
            storage_entry(0, true),
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::Rgba32Float,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            },
        ];
        entries.extend((3..=8).map(|b| storage_entry(b, false)));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vhs-composite"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vhs-composite"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make = |name: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(name),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(name),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let pipelines = KERNELS.iter().map(|name| make(name)).collect();
        let prepare = make(PREPARE_KERNEL);

        let param_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("params"),
            size: PARAM_STRIDE * PARAM_SLOTS as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let signal = (std.field_lines * std.line_samples * 4) as u64;
        let active = (std.field_lines * std.active_width * 4) as u64;
        let lines = (std.field_lines * 32) as u64;
        let sizes = [8 * active, signal, active, 2 * active, lines, 16];
        let names = ["video", "signal", "luma", "detected", "lines", "state"];
        let buffers = std::array::from_fn(|i| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(names[i]),
                size: sizes[i],
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });

        let output = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vhs-output"),
            size: wgpu::Extent3d { width: aw, height: ah, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback_row = (aw * 16).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vhs-readback"),
            size: (readback_row * ah) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Self {
            std,
            params,
            pipelines,
            prepare,
            input_dirty: false,
            layout,
            param_buffer,
            buffers,
            output,
            input: None,
            readback,
            readback_row,
            field_index: 0,
        }
    }

    /// 出力画像の大きさ (有効映像の幅 × フレームの有効ライン数)
    pub fn output_size(&self) -> (usize, usize) {
        self.std.image_size()
    }

    /// 入力画像 (RGBA float、幅 × 高さ × 4) を GPU へ送る
    pub fn upload_input(&mut self, gpu: &GpuContext, rgba: &[f32], width: u32, height: u32) {
        assert_eq!(rgba.len(), (width * height * 4) as usize);
        if self.input.as_ref().is_none_or(|t| t.width() != width || t.height() != height) {
            self.input = Some(gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("vhs-input"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            }));
        }
        let input = self.input.as_ref().unwrap();
        self.input_dirty = true;
        gpu.queue.write_texture(
            input.as_image_copy(),
            bytemuck::cast_slice(rgba),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 16), rows_per_image: None },
            input.size(),
        );
    }

    /// GPU 上のテクスチャ (RGBA32F、TEXTURE_BINDING 付き) をそのまま入力にする。
    /// [`GpuCamera::output_texture`] を渡すと、撮像管カメラの出力をエンコーダに流せる
    /// (`params.input_linear = 0`。カメラの出力は表示用の値で、有効映像と同じ大きさなので縮小しない)。
    /// 中身が毎フィールド変わる前提で、次の `run` で必ず vhsPrepare を流し直す
    pub fn set_input_texture(&mut self, texture: &wgpu::Texture) {
        self.input = Some(texture.clone());
        self.input_dirty = true;
    }

    /// フィールドごとのパラメータを書き、そのスロットを指すバインドグループを作る
    fn prepare_fields(&self, gpu: &GpuContext, fields: usize) -> Vec<wgpu::BindGroup> {
        let input = self.input.as_ref().expect("input uploaded");
        let input_view = input.create_view(&Default::default());
        let output_view = self.output.create_view(&Default::default());
        (0..fields)
            .map(|slot| {
                let index = self.field_index + slot as u64;
                let mut p = self.params;
                p.parity = (index & 1) as u32;
                p.frame_no = (index / 2) as u32;
                p.field_index = index as u32;
                p.src_width = input.width();
                p.src_height = input.height();
                let offset = slot as u64 * PARAM_STRIDE;
                gpu.queue.write_buffer(&self.param_buffer, offset, bytemuck::bytes_of(&p));
                let mut entries = vec![
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.param_buffer,
                            offset,
                            size: wgpu::BufferSize::new(std::mem::size_of::<CompositeParams>() as u64),
                        }),
                    },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&input_view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&output_view) },
                ];
                for (i, b) in self.buffers.iter().enumerate() {
                    entries.push(wgpu::BindGroupEntry { binding: 3 + i as u32, resource: b.as_entire_binding() });
                }
                gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("vhs-composite"),
                    layout: &self.layout,
                    entries: &entries,
                })
            })
            .collect()
    }

    /// RGB(A) float の入力画像 (幅 × 高さ × 4) を `fields` フィールドぶん処理し、出力画像を RGBA float で返す。
    /// 入力は `params.input_linear` に従って表示用の値かシーンリニアとして読む
    pub fn process(&mut self, gpu: &GpuContext, rgba: &[f32], width: u32, height: u32, fields: usize) -> Vec<f32> {
        self.upload_input(gpu, rgba, width, height);
        self.run(gpu, fields);
        self.read_output(gpu)
    }

    /// 送ってある入力で `fields` フィールドぶん進める (読み戻しなし)。
    /// パラメータのスロットぶんずつ 1 回の submit にまとめる
    pub fn run(&mut self, gpu: &GpuContext, fields: usize) {
        self.run_inner(gpu, fields, None);
    }

    /// `run` と同じだが、GPU 上でかかった時間 (ミリ秒) をタイムスタンプで測って返す。
    /// CPU 側のコマンド作成の時間を含まない、エンジンの GPU タイムラインに乗る分の目安
    pub fn run_timed(&mut self, gpu: &GpuContext, fields: usize) -> Option<f64> {
        if !gpu.timestamps {
            return None;
        }
        let batches = fields.div_ceil(PARAM_SLOTS) as u32;
        let count = 2 * batches;
        let query_set = gpu.device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("vhs-run"),
            ty: wgpu::QueryType::Timestamp,
            count,
        });
        self.run_inner(gpu, fields, Some(&query_set));
        let ticks = read_timestamps(gpu, &query_set, count)?;
        let period = gpu.queue.get_timestamp_period() as f64;
        let total: u64 = ticks.chunks(2).map(|t| t[1].saturating_sub(t[0])).sum();
        Some(total as f64 * period * 1e-6)
    }

    fn run_inner(&mut self, gpu: &GpuContext, fields: usize, timing: Option<&wgpu::QuerySet>) {
        let mut left = fields;
        let mut batch_no = 0u32;
        while left > 0 {
            let batch = left.min(PARAM_SLOTS);
            let groups = self.prepare_fields(gpu, batch);
            let mut cmd = gpu.device.create_command_encoder(&Default::default());
            {
                let mut pass = cmd.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("vhs-composite"),
                    timestamp_writes: timing.map(|query_set| wgpu::ComputePassTimestampWrites {
                        query_set,
                        beginning_of_pass_write_index: Some(2 * batch_no),
                        end_of_pass_write_index: Some(2 * batch_no + 1),
                    }),
                });
                if self.input_dirty {
                    // 入力の縮小・色変換は入力 1 枚につき 1 回 (フィールドに依らない)
                    pass.set_bind_group(0, &groups[0], &[]);
                    pass.set_pipeline(&self.prepare);
                    let (x, y, z) = dispatch_size(PREPARE_KERNEL, self.std);
                    pass.dispatch_workgroups(x, y, z);
                    self.input_dirty = false;
                }
                for group in &groups {
                    pass.set_bind_group(0, group, &[]);
                    for (pipeline, name) in self.pipelines.iter().zip(KERNELS) {
                        pass.set_pipeline(pipeline);
                        let (x, y, z) = dispatch_size(name, self.std);
                        pass.dispatch_workgroups(x, y, z);
                    }
                }
            }
            gpu.queue.submit([cmd.finish()]);
            self.field_index += batch as u64;
            left -= batch;
            batch_no += 1;
        }
    }

    /// 処理が終わるまで待つ (時間を測るとき用)
    pub fn wait(&self, gpu: &GpuContext) {
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU wait failed");
    }

    /// 1 フィールドぶんを、カーネルごとに別のパスにしてタイムスタンプを取る。
    /// 先頭は入力 1 枚ぶんの vhsPrepare (フィールドごとにはかからない)。
    /// 返り値は (カーネル名, ミリ秒)。TIMESTAMP_QUERY が使えなければ None
    pub fn profile_field(&mut self, gpu: &GpuContext) -> Option<Vec<(&'static str, f64)>> {
        if !gpu.timestamps {
            return None;
        }
        let names: Vec<&'static str> = std::iter::once(PREPARE_KERNEL).chain(KERNELS).collect();
        let pipelines: Vec<&wgpu::ComputePipeline> = std::iter::once(&self.prepare).chain(&self.pipelines).collect();
        let count = names.len() as u32 * 2;
        let query_set = gpu.device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("vhs-profile"),
            ty: wgpu::QueryType::Timestamp,
            count,
        });
        let groups = self.prepare_fields(gpu, 1);
        let mut cmd = gpu.device.create_command_encoder(&Default::default());
        for (k, (pipeline, name)) in pipelines.iter().zip(&names).enumerate() {
            let mut pass = cmd.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(name),
                timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                    query_set: &query_set,
                    beginning_of_pass_write_index: Some(2 * k as u32),
                    end_of_pass_write_index: Some(2 * k as u32 + 1),
                }),
            });
            pass.set_bind_group(0, &groups[0], &[]);
            pass.set_pipeline(pipeline);
            let (x, y, z) = dispatch_size(name, self.std);
            pass.dispatch_workgroups(x, y, z);
        }
        gpu.queue.submit([cmd.finish()]);
        self.field_index += 1;
        let ticks = read_timestamps(gpu, &query_set, count)?;
        let period = gpu.queue.get_timestamp_period() as f64;
        let result = names
            .iter()
            .enumerate()
            .map(|(k, name)| (*name, ticks[2 * k + 1].saturating_sub(ticks[2 * k]) as f64 * period * 1e-6))
            .collect();
        Some(result)
    }

    /// 出力テクスチャを読み戻す (RGBA float、有効映像の幅 × フレームのライン数 × 4)
    pub fn read_output(&self, gpu: &GpuContext) -> Vec<f32> {
        let (aw, ah) = self.std.image_size();
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
        let mut out = Vec::with_capacity(aw * ah * 4);
        {
            let data = slice.get_mapped_range().expect("mapped range");
            for y in 0..ah {
                let start = y * self.readback_row as usize;
                out.extend_from_slice(bytemuck::cast_slice(&data[start..start + aw * 16]));
            }
        }
        self.readback.unmap();
        out
    }
}

/// タイムスタンプのクエリを読み戻す
fn read_timestamps(gpu: &GpuContext, query_set: &wgpu::QuerySet, count: u32) -> Option<Vec<u64>> {
    let size = count as u64 * 8;
    let resolve = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("vhs-timestamps-resolve"),
        size,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("vhs-timestamps-read"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut cmd = gpu.device.create_command_encoder(&Default::default());
    cmd.resolve_query_set(query_set, 0..count, &resolve, 0);
    cmd.copy_buffer_to_buffer(&resolve, 0, &read, 0, size);
    gpu.queue.submit([cmd.finish()]);
    let slice = read.slice(..);
    let (tx, rx) = channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
    rx.recv().ok()?.ok()?;
    let data = slice.get_mapped_range().ok()?;
    Some(bytemuck::cast_slice::<u8, u64>(&data).to_vec())
}


/// 比較テスト・エンジン側の確認用の共通部品 (テストパターンと CPU 版の参照出力)
pub mod reference {
    use vhs_core::decoder::Decoder;
    use vhs_core::encoder::encode_field;
    use vhs_core::frame::{Field, RgbaFrame, VideoFrame};
    use vhs_core::params::{DecoderParams, EncoderParams};
    use vhs_core::TvStandard;

    /// カラーバー・ランプ・細かい縦縞・斜めの境界を並べたパターン (表示用の値、0..1)
    pub fn test_pattern(w: usize, h: usize) -> Vec<[f32; 3]> {
        const BARS: [[f32; 3]; 8] = [
            [0.75, 0.75, 0.75],
            [0.75, 0.75, 0.0],
            [0.0, 0.75, 0.75],
            [0.0, 0.75, 0.0],
            [0.75, 0.0, 0.75],
            [0.75, 0.0, 0.0],
            [0.0, 0.0, 0.75],
            [0.0, 0.0, 0.0],
        ];
        let mut px = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let u = x as f32 / w as f32;
                let c = match y * 4 / h {
                    0 => BARS[(x * 8 / w).min(7)],
                    1 => [u, 1.0 - u, (u * 6.0).fract()],
                    2 => {
                        let v = if (x / (1 + y % 7)) % 2 == 0 { 0.9 } else { 0.1 };
                        [v, v * 0.8, 0.2]
                    }
                    _ => {
                        if x + y < w { [0.2, 0.4, 0.9] } else { [0.95, 0.6, 0.1] }
                    }
                };
                px.push(c);
            }
        }
        px
    }

    /// CPU 版 (vhs-core) でエンコード → デコードを `fields` フィールドぶん回し、最後の出力を返す
    pub fn cpu_decode(
        standard: TvStandard,
        enc: &EncoderParams,
        dec: &DecoderParams,
        px: &[[f32; 3]],
        fields: usize,
    ) -> RgbaFrame {
        let std = standard.spec();
        let (w, h) = std.image_size();
        let mut frame = VideoFrame::new(w, h);
        for (i, c) in px.iter().enumerate() {
            frame.data[i * 3..i * 3 + 3].copy_from_slice(c);
        }
        let mut field = Field::new(std);
        let mut decoder = Decoder::new(std);
        let mut out = RgbaFrame::new(w, h);
        for index in 0..fields as u64 {
            field.set_index(index);
            encode_field(&frame, &mut field, enc);
            decoder.decode_field(&field, dec, false, &mut out);
        }
        out
    }
}
