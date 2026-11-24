//! Small C ABI for render hosts such as Blender.
//!
//! The host passes one scene-linear, float32 RGBA image for each rendered frame.
//! A processor keeps VHS state between calls and advances the tape at the
//! requested render frame rate.

use std::ffi::c_void;
use std::slice;
use vhs_core::{LinearFrame, Params, Pipeline};

const OK: i32 = 0;
const ERR_ARGUMENT: i32 = -1;
const ERR_BUFFER: i32 = -2;
const ERR_FRAME_SIZE: i32 = -3;
const ERR_ALPHA_MODE: i32 = -4;

struct Processor {
    pipeline: Pipeline,
    fps_num: u32,
    fps_den: u32,
    /// Fractional fields carried into the next host frame (fixed-point numerator).
    field_remainder: u128,
    frames_processed: u64,
    last_error: String,
}

/// Create a stateful processor from a serialized `vhs_core::Params` value.
///
/// `fps_num / fps_den` is the host render frame rate. On failure this returns
/// null and writes a UTF-8 error message into `error_buffer` when provided.
///
/// # Safety
/// `params_json` must point to `params_json_len` readable bytes unless the
/// length is zero. If `error_capacity` is nonzero, `error_buffer` must point to
/// that many writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vhs_processor_new(
    params_json: *const u8,
    params_json_len: usize,
    fps_num: u32,
    fps_den: u32,
    error_buffer: *mut u8,
    error_capacity: usize,
) -> *mut c_void {
    if fps_num == 0 || fps_den == 0 {
        unsafe {
            write_error(
                error_buffer,
                error_capacity,
                "render frame rate must be positive",
            )
        };
        return std::ptr::null_mut();
    }
    if params_json.is_null() && params_json_len != 0 {
        unsafe { write_error(error_buffer, error_capacity, "params_json is null") };
        return std::ptr::null_mut();
    }

    // SAFETY: the caller provides a readable byte range of params_json_len bytes.
    let bytes = if params_json_len == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(params_json, params_json_len) }
    };
    let params: Params = match serde_json::from_slice(bytes) {
        Ok(params) => params,
        Err(error) => {
            unsafe {
                write_error(
                    error_buffer,
                    error_capacity,
                    &format!("invalid params JSON: {error}"),
                )
            };
            return std::ptr::null_mut();
        }
    };

    let pipeline = Pipeline::new(params);
    let (_, field_rate_den) = pipeline.standard().field_rate_ratio;
    let fields_denominator = field_rate_den as u128 * fps_num as u128;
    let processor = Processor {
        pipeline,
        fps_num,
        fps_den,
        // Round the first frame to the nearest field count so it normally
        // contains both interlaced parities instead of starting half blank.
        field_remainder: fields_denominator / 2,
        frames_processed: 0,
        last_error: String::new(),
    };
    Box::into_raw(Box::new(processor)).cast()
}

/// Destroy a processor returned by `vhs_processor_new`.
///
/// # Safety
/// `handle` must be null or a live handle returned by `vhs_processor_new` that
/// has not already been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vhs_processor_free(handle: *mut c_void) {
    if !handle.is_null() {
        // SAFETY: handle must be a live value returned by vhs_processor_new.
        drop(unsafe { Box::from_raw(handle.cast::<Processor>()) });
    }
}

/// Return the last error message as UTF-8, including a trailing NUL when it
/// fits. The return value is the required capacity including that NUL.
///
/// # Safety
/// `handle` must be null or a live processor handle. If `error_capacity` is
/// nonzero, `error_buffer` must point to that many writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vhs_processor_last_error(
    handle: *const c_void,
    error_buffer: *mut u8,
    error_capacity: usize,
) -> usize {
    if handle.is_null() {
        return unsafe { write_error(error_buffer, error_capacity, "processor handle is null") };
    }
    // SAFETY: handle must be a live value returned by vhs_processor_new.
    let processor = unsafe { &*handle.cast::<Processor>() };
    unsafe { write_error(error_buffer, error_capacity, &processor.last_error) }
}

/// Process one Blender/render-host frame from scene-linear float32 RGBA.
///
/// `alpha_mode` is 0 for straight alpha or 1 for premultiplied alpha. Output
/// dimensions match the input dimensions. RGB output is scene-linear float32;
/// alpha is preserved from the input. Input and output buffers each contain
/// `width * height * 4` float samples.
///
/// # Safety
/// `handle` must be a live processor handle. `input` must point to
/// `input_samples` readable floats and `output` to `output_samples` writable
/// floats. The input and output ranges must not overlap.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vhs_processor_process_rgba_f32(
    handle: *mut c_void,
    input: *const f32,
    input_samples: usize,
    output: *mut f32,
    output_samples: usize,
    width: usize,
    height: usize,
    alpha_mode: u32,
) -> i32 {
    if handle.is_null() {
        return ERR_ARGUMENT;
    }
    // SAFETY: handle is checked above and must be a live processor pointer.
    let processor = unsafe { &mut *handle.cast::<Processor>() };
    processor.last_error.clear();

    let Some(pixel_count) = width.checked_mul(height) else {
        return fail(processor, ERR_FRAME_SIZE, "frame dimensions overflow");
    };
    let Some(required_samples) = pixel_count.checked_mul(4) else {
        return fail(processor, ERR_FRAME_SIZE, "frame sample count overflow");
    };
    if width == 0 || height == 0 || input_samples != required_samples {
        return fail(
            processor,
            ERR_FRAME_SIZE,
            "input must contain width * height * 4 float samples",
        );
    }
    if output_samples < required_samples {
        return fail(processor, ERR_BUFFER, "output buffer is too small");
    }
    if input.is_null() || output.is_null() {
        return fail(
            processor,
            ERR_ARGUMENT,
            "input and output pointers must be non-null",
        );
    }
    if alpha_mode > 1 {
        return fail(
            processor,
            ERR_ALPHA_MODE,
            "alpha_mode must be 0 (straight) or 1 (premultiplied)",
        );
    }

    // SAFETY: the caller provides readable/writable ranges of the validated sizes.
    let input = unsafe { slice::from_raw_parts(input, required_samples) };
    let output = unsafe { slice::from_raw_parts_mut(output, required_samples) };
    let mut rgb = Vec::with_capacity(pixel_count * 3);
    for rgba in input.chunks_exact(4) {
        let alpha = finite_unit(rgba[3]);
        for &channel in &rgba[..3] {
            let channel = finite_nonnegative(channel);
            let linear = if alpha <= 1e-8 {
                0.0
            } else if alpha_mode == 1 {
                channel / alpha
            } else {
                channel
            };
            rgb.push(finite_nonnegative(linear));
        }
    }
    let Some(frame) = LinearFrame::from_raw(width, height, rgb) else {
        return fail(
            processor,
            ERR_FRAME_SIZE,
            "failed to create linear input frame",
        );
    };

    let (field_rate_num, field_rate_den) = processor.pipeline.standard().field_rate_ratio;
    let fields_numerator = field_rate_num as u128 * processor.fps_den as u128;
    let fields_denominator = field_rate_den as u128 * processor.fps_num as u128;
    processor.field_remainder += fields_numerator;
    let mut fields = (processor.field_remainder / fields_denominator) as usize;
    processor.field_remainder %= fields_denominator;
    // Even when the host asks for more frames per second than the tape can
    // produce fields, initialize the first output from the first input frame.
    if processor.frames_processed == 0 && fields == 0 {
        fields = 1;
    }
    for _ in 0..fields {
        processor.pipeline.process_field_linear(&frame);
    }

    let decoded = processor.pipeline.output();
    for y in 0..height {
        let fy = ((y as f32 + 0.5) * decoded.height as f32 / height as f32 - 0.5)
            .clamp(0.0, (decoded.height - 1) as f32);
        let y0 = fy.floor() as usize;
        let y1 = (y0 + 1).min(decoded.height - 1);
        let wy = fy - y0 as f32;
        for x in 0..width {
            let fx = ((x as f32 + 0.5) * decoded.width as f32 / width as f32 - 0.5)
                .clamp(0.0, (decoded.width - 1) as f32);
            let x0 = fx.floor() as usize;
            let x1 = (x0 + 1).min(decoded.width - 1);
            let wx = fx - x0 as f32;
            let dst = (y * width + x) * 4;
            let src_alpha = finite_unit(input[dst + 3]);
            for channel in 0..3 {
                let sample = |sx: usize, sy: usize| {
                    decoded.data[(sy * decoded.width + sx) * 4 + channel] as f32
                };
                let top = sample(x0, y0) * (1.0 - wx) + sample(x1, y0) * wx;
                let bottom = sample(x0, y1) * (1.0 - wx) + sample(x1, y1) * wx;
                // The decoder stores display-coded 8-bit video. Convert back
                // to the scene-linear convention expected by render hosts.
                let mut linear = ((top * (1.0 - wy) + bottom * wy) / 255.0).powf(2.2);
                if alpha_mode == 1 {
                    linear *= src_alpha;
                }
                output[dst + channel] = linear;
            }
            output[dst + 3] = src_alpha;
        }
    }
    processor.frames_processed += 1;
    OK
}

fn finite_nonnegative(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn finite_unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn fail(processor: &mut Processor, code: i32, message: &str) -> i32 {
    processor.last_error.clear();
    processor.last_error.push_str(message);
    code
}

unsafe fn write_error(buffer: *mut u8, capacity: usize, message: &str) -> usize {
    let required = message.len() + 1;
    if !buffer.is_null() && capacity > 0 {
        let count = message.len().min(capacity - 1);
        // SAFETY: caller supplies writable buffer of `capacity` bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(message.as_ptr(), buffer, count);
            *buffer.add(count) = 0;
        }
    }
    required
}
