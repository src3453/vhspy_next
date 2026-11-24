"""Blender post-render bridge for vhs-ffi.

Install this file as a Blender add-on, build `vhs-ffi`, then configure the
native library path in the Render properties panel.
"""

bl_info = {
    "name": "VHS Next Render Pass",
    "author": "VHS Next contributors",
    "version": (0, 1, 0),
    "blender": (4, 2, 0),
    "location": "Render Properties > VHS Next",
    "description": "Process Blender render results through the VHS signal simulator",
    "category": "Render",
}

import array
import ctypes
import math
import os

import bpy
from bpy.app.handlers import persistent
from bpy.props import BoolProperty, EnumProperty, StringProperty


_OUTPUT_IMAGE_NAME = "VHS Pass"
_ACTIVE_RUN = None


def _scene_rate(scene):
    # Blender stores a nominal integer FPS and a scale such as 1.001.
    fps_num = int(scene.render.fps) * 1000
    fps_den = max(1, int(round(float(scene.render.fps_base) * 1000)))
    divisor = math.gcd(fps_num, fps_den)
    return fps_num // divisor, fps_den // divisor


def _configure_library(library):
    byte_ptr = ctypes.POINTER(ctypes.c_uint8)
    float_ptr = ctypes.POINTER(ctypes.c_float)

    library.vhs_processor_new.argtypes = [
        byte_ptr,
        ctypes.c_size_t,
        ctypes.c_uint32,
        ctypes.c_uint32,
        byte_ptr,
        ctypes.c_size_t,
    ]
    library.vhs_processor_new.restype = ctypes.c_void_p
    library.vhs_processor_free.argtypes = [ctypes.c_void_p]
    library.vhs_processor_free.restype = None
    library.vhs_processor_last_error.argtypes = [ctypes.c_void_p, byte_ptr, ctypes.c_size_t]
    library.vhs_processor_last_error.restype = ctypes.c_size_t
    library.vhs_processor_process_rgba_f32.argtypes = [
        ctypes.c_void_p,
        float_ptr,
        ctypes.c_size_t,
        float_ptr,
        ctypes.c_size_t,
        ctypes.c_size_t,
        ctypes.c_size_t,
        ctypes.c_uint32,
    ]
    library.vhs_processor_process_rgba_f32.restype = ctypes.c_int32
    return library


class _NativeProcessor:
    def __init__(self, scene):
        library_path = bpy.path.abspath(scene.vhs_next_library_path)
        if not library_path or not os.path.isfile(library_path):
            raise RuntimeError("Set the path to vhs_ffi.dll (or the platform library) first")

        self.library = _configure_library(ctypes.CDLL(library_path))
        params_path = bpy.path.abspath(scene.vhs_next_params_path) if scene.vhs_next_params_path else ""
        if params_path:
            with open(params_path, "rb") as params_file:
                params = params_file.read()
        else:
            params = b"{}"
        params_buffer = (ctypes.c_uint8 * len(params)).from_buffer_copy(params)
        error_buffer = ctypes.create_string_buffer(1024)
        fps_num, fps_den = _scene_rate(scene)
        self.alpha_mode = 1 if scene.vhs_next_alpha_mode == "PREMUL" else 0
        self.handle = self.library.vhs_processor_new(
            params_buffer,
            len(params),
            fps_num,
            fps_den,
            ctypes.cast(error_buffer, ctypes.POINTER(ctypes.c_uint8)),
            len(error_buffer),
        )
        if not self.handle:
            message = error_buffer.value.decode("utf-8", errors="replace")
            raise RuntimeError(message or "Native VHS processor could not be created")

    def process(self, render_result):
        width, height = (int(render_result.size[0]), int(render_result.size[1]))
        if width <= 0 or height <= 0:
            raise RuntimeError("Render Result has no pixels")

        samples = width * height * 4
        source = array.array("f", [0.0]) * samples
        output = array.array("f", [0.0]) * samples
        render_result.pixels.foreach_get(source)
        source_ptr = ctypes.cast(ctypes.c_void_p(source.buffer_info()[0]), ctypes.POINTER(ctypes.c_float))
        output_ptr = ctypes.cast(ctypes.c_void_p(output.buffer_info()[0]), ctypes.POINTER(ctypes.c_float))
        result = self.library.vhs_processor_process_rgba_f32(
            self.handle,
            source_ptr,
            samples,
            output_ptr,
            samples,
            width,
            height,
            self.alpha_mode,
        )
        if result != 0:
            error_buffer = ctypes.create_string_buffer(1024)
            self.library.vhs_processor_last_error(
                self.handle,
                ctypes.cast(error_buffer, ctypes.POINTER(ctypes.c_uint8)),
                len(error_buffer),
            )
            message = error_buffer.value.decode("utf-8", errors="replace")
            raise RuntimeError(message or f"VHS processor failed with status {result}")
        return width, height, output

    def close(self):
        if self.handle:
            self.library.vhs_processor_free(self.handle)
            self.handle = None


def _set_output_image(width, height, pixels, alpha_mode):
    image = bpy.data.images.get(_OUTPUT_IMAGE_NAME)
    if image and tuple(image.size) != (width, height):
        bpy.data.images.remove(image)
        image = None
    if image is None:
        image = bpy.data.images.new(_OUTPUT_IMAGE_NAME, width=width, height=height, alpha=True, float_buffer=True)

    image.alpha_mode = "PREMUL" if alpha_mode == "PREMUL" else "STRAIGHT"
    image.colorspace_settings.name = "Linear Rec.709"
    image.pixels.foreach_set(pixels)
    image.update()
    return image


def _save_openexr(image, scene, path):
    settings = scene.render.image_settings
    previous = {
        "file_format": settings.file_format,
        "color_mode": settings.color_mode,
        "color_depth": settings.color_depth,
        "color_management": settings.color_management,
        "exr_codec": settings.exr_codec,
    }
    try:
        settings.file_format = "OPEN_EXR"
        settings.color_mode = "RGBA"
        settings.color_depth = "32"
        settings.color_management = "OVERRIDE"
        settings.exr_codec = "ZIP"
        image.save_render(path, scene=scene)
    finally:
        settings.file_format = previous["file_format"]
        settings.color_mode = previous["color_mode"]
        settings.color_depth = previous["color_depth"]
        settings.color_management = previous["color_management"]
        settings.exr_codec = previous["exr_codec"]


def _process_result(processor, scene, save_sequence_frame=False):
    render_result = bpy.data.images.get("Render Result")
    if render_result is None:
        raise RuntimeError("Render Result is not available")
    width, height, pixels = processor.process(render_result)
    _set_output_image(width, height, pixels, scene.vhs_next_alpha_mode)

    if save_sequence_frame and scene.vhs_next_output_dir:
        directory = bpy.path.abspath(scene.vhs_next_output_dir)
        os.makedirs(directory, exist_ok=True)
        filename = f"vhs_{scene.frame_current:04d}.exr"
        _save_openexr(bpy.data.images[_OUTPUT_IMAGE_NAME], scene, os.path.join(directory, filename))
    return width, height


@persistent
def _render_init(scene):
    global _ACTIVE_RUN
    if not scene.vhs_next_auto_process:
        return
    _close_active_run()
    try:
        if scene.frame_step != 1:
            raise RuntimeError("Animation VHS processing requires a render frame step of 1")
        _ACTIVE_RUN = {
            "processor": _NativeProcessor(scene),
            "next_frame": scene.frame_current,
        }
        scene.vhs_next_status = "VHS pass is ready for the render sequence"
    except Exception as error:
        _ACTIVE_RUN = None
        scene.vhs_next_status = str(error)
        print(f"VHS Next: {error}")


@persistent
def _render_write(scene):
    if not scene.vhs_next_auto_process:
        return
    if _ACTIVE_RUN is None:
        return
    try:
        if scene.frame_current != _ACTIVE_RUN["next_frame"]:
            _ACTIVE_RUN["processor"].close()
            _ACTIVE_RUN["processor"] = _NativeProcessor(scene)
            scene.vhs_next_status = "Frame sequence changed; VHS state was reset"
        width, height = _process_result(_ACTIVE_RUN["processor"], scene, save_sequence_frame=True)
        _ACTIVE_RUN["next_frame"] = scene.frame_current + 1
        scene.vhs_next_status = f"Processed frame {scene.frame_current}: {width} × {height}"
    except Exception as error:
        scene.vhs_next_status = str(error)
        print(f"VHS Next: {error}")
        _close_active_run()


@persistent
def _render_finished(_scene):
    _close_active_run()


def _close_active_run():
    global _ACTIVE_RUN
    if _ACTIVE_RUN is not None:
        _ACTIVE_RUN["processor"].close()
        _ACTIVE_RUN = None


class VHSNEXT_OT_process_render_result(bpy.types.Operator):
    bl_idname = "vhs_next.process_render_result"
    bl_label = "Process Current Render Result"
    bl_description = "Process the current Render Result and create a float image named VHS Pass"
    bl_options = {"REGISTER"}

    def execute(self, context):
        processor = None
        try:
            processor = _NativeProcessor(context.scene)
            width, height = _process_result(processor, context.scene)
            context.scene.vhs_next_status = f"Processed current result: {width} × {height}"
            self.report({"INFO"}, context.scene.vhs_next_status)
            return {"FINISHED"}
        except Exception as error:
            context.scene.vhs_next_status = str(error)
            self.report({"ERROR"}, str(error))
            return {"CANCELLED"}
        finally:
            if processor is not None:
                processor.close()


class RENDER_PT_vhs_next(bpy.types.Panel):
    bl_label = "VHS Next"
    bl_idname = "RENDER_PT_vhs_next"
    bl_space_type = "PROPERTIES"
    bl_region_type = "WINDOW"
    bl_context = "render"

    @classmethod
    def poll(cls, context):
        return context.scene is not None

    def draw(self, context):
        layout = self.layout
        scene = context.scene
        layout.prop(scene, "vhs_next_library_path", text="Native library")
        layout.prop(scene, "vhs_next_params_path", text="Params JSON")
        layout.prop(scene, "vhs_next_alpha_mode", text="Input alpha")
        layout.operator(VHSNEXT_OT_process_render_result.bl_idname, icon="IMAGE_DATA")
        layout.separator()
        layout.prop(scene, "vhs_next_auto_process", text="Process animation renders")
        if scene.vhs_next_auto_process:
            layout.prop(scene, "vhs_next_output_dir", text="EXR sequence folder")
        if scene.vhs_next_status:
            layout.label(text=scene.vhs_next_status, icon="INFO")


_CLASSES = (VHSNEXT_OT_process_render_result, RENDER_PT_vhs_next)


def register():
    for cls in _CLASSES:
        bpy.utils.register_class(cls)
    bpy.types.Scene.vhs_next_library_path = StringProperty(
        name="Native library",
        description="Path to vhs_ffi.dll, libvhs_ffi.so, or libvhs_ffi.dylib",
        subtype="FILE_PATH",
    )
    bpy.types.Scene.vhs_next_params_path = StringProperty(
        name="Params JSON",
        description="Optional VHS Next parameter JSON; blank uses core defaults",
        subtype="FILE_PATH",
    )
    bpy.types.Scene.vhs_next_alpha_mode = EnumProperty(
        name="Input alpha",
        description="Alpha convention used by the Blender render result",
        items=(
            ("PREMUL", "Premultiplied", "Unpremultiply before VHS processing, then restore premultiplication"),
            ("STRAIGHT", "Straight", "Pass straight RGB values to the VHS processor"),
        ),
        default="PREMUL",
    )
    bpy.types.Scene.vhs_next_auto_process = BoolProperty(
        name="Process animation renders",
        description="Process each sequential render result and write float OpenEXR frames",
        default=False,
    )
    bpy.types.Scene.vhs_next_output_dir = StringProperty(
        name="EXR sequence folder",
        description="Directory for vhs_####.exr frames when animation processing is enabled",
        subtype="DIR_PATH",
        default="//vhs_pass/",
    )
    bpy.types.Scene.vhs_next_status = StringProperty(name="VHS Next status", default="")

    bpy.app.handlers.render_init.append(_render_init)
    bpy.app.handlers.render_write.append(_render_write)
    bpy.app.handlers.render_complete.append(_render_finished)
    bpy.app.handlers.render_cancel.append(_render_finished)


def unregister():
    _close_active_run()
    for handlers, callback in (
        (bpy.app.handlers.render_init, _render_init),
        (bpy.app.handlers.render_write, _render_write),
        (bpy.app.handlers.render_complete, _render_finished),
        (bpy.app.handlers.render_cancel, _render_finished),
    ):
        if callback in handlers:
            handlers.remove(callback)
    for name in (
        "vhs_next_library_path",
        "vhs_next_params_path",
        "vhs_next_alpha_mode",
        "vhs_next_auto_process",
        "vhs_next_output_dir",
        "vhs_next_status",
    ):
        if hasattr(bpy.types.Scene, name):
            delattr(bpy.types.Scene, name)
    for cls in reversed(_CLASSES):
        bpy.utils.unregister_class(cls)


if __name__ == "__main__":
    register()
