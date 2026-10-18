use std::rc::Rc;
use std::sync::Arc;

use kaplay_native_canvas2d::ImageSource;
use kaplay_native_gfx::pixels::UploadOptions;
use v8::{FunctionCallbackArguments, Local, PinScope, ReturnValue, Value};

use super::{handle, option_handle, parse_headers, source_size};
use crate::state::{HostState, VideoInfo};

type Args<'s> = FunctionCallbackArguments<'s>;
type Rv<'s> = ReturnValue<'s, Value>;

fn host<'s>(args: &Args<'s>) -> &'s HostState {
    let external = Local::<v8::External>::try_from(args.data()).expect("native state");
    unsafe { &*(external.value() as *const HostState) }
}

fn f64_arg(scope: &PinScope<'_, '_>, args: &Args<'_>, index: i32) -> f64 {
    args.get(index).number_value(scope).unwrap_or(0.0)
}

fn i32_arg(scope: &PinScope<'_, '_>, args: &Args<'_>, index: i32) -> i32 {
    args.get(index).int32_value(scope).unwrap_or(0)
}

fn u32_arg(scope: &PinScope<'_, '_>, args: &Args<'_>, index: i32) -> u32 {
    args.get(index).uint32_value(scope).unwrap_or(0)
}

fn bool_arg(scope: &PinScope<'_, '_>, args: &Args<'_>, index: i32) -> bool {
    args.get(index).boolean_value(scope)
}

fn string_arg(scope: &PinScope<'_, '_>, args: &Args<'_>, index: i32) -> String {
    args.get(index).to_rust_string_lossy(scope)
}

fn with_u8<R>(value: Local<Value>, run: impl FnOnce(&[u8]) -> R) -> Option<R> {
    let view = Local::<v8::ArrayBufferView>::try_from(value).ok()?;
    let backing = view.get_backing_store()?;
    let pointer = view.data() as *const u8;
    let length = view.byte_length();
    let result = if length == 0 {
        run(&[])
    } else {
        let slice = unsafe { std::slice::from_raw_parts(pointer, length) };
        run(slice)
    };
    drop(backing);
    Some(result)
}

fn with_u8_mut<R>(value: Local<Value>, run: impl FnOnce(&mut [u8]) -> R) -> Option<R> {
    let view = Local::<v8::ArrayBufferView>::try_from(value).ok()?;
    let backing = view.get_backing_store()?;
    let pointer = view.data() as *mut u8;
    let length = view.byte_length();
    let result = if length == 0 {
        run(&mut [])
    } else {
        let slice = unsafe { std::slice::from_raw_parts_mut(pointer, length) };
        run(slice)
    };
    drop(backing);
    Some(result)
}

fn with_f32<R>(value: Local<Value>, run: impl FnOnce(&[f32]) -> R) -> Option<R> {
    let view = Local::<v8::ArrayBufferView>::try_from(value).ok()?;
    let backing = view.get_backing_store()?;
    let pointer = view.data();
    let length = view.byte_length();
    let result = if length == 0 {
        run(&[])
    } else if !length.is_multiple_of(4) || !(pointer as usize).is_multiple_of(4) {
        return None;
    } else {
        let slice = unsafe { std::slice::from_raw_parts(pointer as *const f32, length / 4) };
        run(slice)
    };
    drop(backing);
    Some(result)
}

fn with_f32_mut<R>(value: Local<Value>, run: impl FnOnce(&mut [f32]) -> R) -> Option<R> {
    let view = Local::<v8::ArrayBufferView>::try_from(value).ok()?;
    let backing = view.get_backing_store()?;
    let pointer = view.data();
    let length = view.byte_length();
    let result = if length == 0 {
        run(&mut [])
    } else if !length.is_multiple_of(4) || !(pointer as usize).is_multiple_of(4) {
        return None;
    } else {
        let slice = unsafe { std::slice::from_raw_parts_mut(pointer as *mut f32, length / 4) };
        run(slice)
    };
    drop(backing);
    Some(result)
}

fn set_string(scope: &PinScope<'_, '_>, rv: &mut Rv<'_>, value: &str) {
    match v8::String::new(scope, value) {
        Some(text) => rv.set(text.into()),
        None => rv.set_undefined(),
    }
}

fn set_opt_string(scope: &PinScope<'_, '_>, rv: &mut Rv<'_>, value: Option<String>) {
    let text = value.and_then(|value| v8::String::new(scope, &value));
    match text {
        Some(text) => rv.set(text.into()),
        None => rv.set_undefined(),
    }
}

fn set_f64_array(scope: &PinScope<'_, '_>, rv: &mut Rv<'_>, values: &[f64]) {
    let array = v8::Array::new(scope, values.len() as i32);
    for (index, value) in values.iter().enumerate() {
        let _ = array.set_index(scope, index as u32, v8::Number::new(scope, *value).into());
    }
    rv.set(array.into());
}

fn register<'s>(
    scope: &PinScope<'s, '_>,
    object: Local<'s, v8::Object>,
    name: &str,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
    data: Local<'s, Value>,
) {
    let function = v8::Function::builder(callback)
        .data(data)
        .build(scope)
        .expect("native function");
    let key = v8::String::new(scope, name).expect("native function name");
    let _ = object.set(scope, key.into(), function.into());
}

fn set_property<'s>(
    scope: &PinScope<'s, '_>,
    object: Local<'s, v8::Object>,
    name: &str,
    value: Local<'s, Value>,
) {
    let key = v8::String::new(scope, name).expect("native property name");
    let _ = object.set(scope, key.into(), value);
}

pub fn install(
    scope: &mut PinScope<'_, '_>,
    state: &Rc<HostState>,
    shim: &str,
) -> Result<(), String> {
    let data: Local<Value> =
        v8::External::new(scope, Rc::as_ptr(state) as *mut std::ffi::c_void).into();
    let native = v8::Object::new(scope);
    install_env(scope, native, data, state);
    install_window(scope, native, data);
    install_fs(scope, native, data);
    install_storage(scope, native, data);
    install_image(scope, native, data);
    install_font(scope, native, data);
    install_canvas(scope, native, data);
    install_gl(scope, native, data);
    install_net(scope, native, data);
    install_auth(scope, native, data);
    install_audio(scope, native, data);
    install_video(scope, native, data);
    install_steam(scope, native, data);
    install_updates(scope, native, data);

    let global = scope.get_current_context().global(scope);
    set_property(scope, global, "__kapNative", native.into());

    let source = v8::String::new(scope, shim)
        .ok_or_else(|| "failed to allocate the shim source".to_string())?;
    v8::tc_scope!(let caught, scope);
    if let Some(script) = v8::Script::compile(caught, source, None) {
        script.run(caught);
    }
    if caught.has_caught() {
        let message = caught
            .exception()
            .map(|value| value.to_rust_string_lossy(caught))
            .unwrap_or_default();
        return Err(message);
    }
    Ok(())
}

fn install_env(
    scope: &PinScope<'_, '_>,
    native: Local<v8::Object>,
    data: Local<Value>,
    state: &Rc<HostState>,
) {
    let object = v8::Object::new(scope);
    set_property(
        scope,
        object,
        "hostVersion",
        v8::String::new(scope, state.host_version).unwrap().into(),
    );
    set_property(
        scope,
        object,
        "platform",
        v8::String::new(scope, state.platform).unwrap().into(),
    );
    set_property(
        scope,
        object,
        "gameName",
        v8::String::new(scope, &state.game_name).unwrap().into(),
    );
    set_property(
        scope,
        object,
        "networkAllowed",
        v8::Boolean::new(scope, state.allow_network).into(),
    );
    set_property(
        scope,
        object,
        "startUrl",
        v8::String::new(scope, &state.start_url).unwrap().into(),
    );
    register(scope, object, "now", env_now, data);
    register(scope, object, "log", env_log, data);
    register(scope, object, "quit", env_quit, data);
    set_property(scope, native, "env", object.into());
}

fn env_now<'s, 'i>(_scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    rv.set_double(host(&args).now_ms());
}

fn env_log<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let level = string_arg(scope, &args, 0);
    let message = string_arg(scope, &args, 1);
    match level.as_str() {
        "error" => log::error!("{message}"),
        "warn" => log::warn!("{message}"),
        _ => log::info!("{message}"),
    }
}

fn env_quit<'s, 'i>(_scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    host(&args).quit.set(true);
}

fn install_window(scope: &PinScope<'_, '_>, native: Local<v8::Object>, data: Local<Value>) {
    let object = v8::Object::new(scope);
    register(scope, object, "size", window_size, data);
    register(scope, object, "setTitle", window_set_title, data);
    register(scope, object, "setFullscreen", window_set_fullscreen, data);
    register(scope, object, "isFullscreen", window_is_fullscreen, data);
    register(scope, object, "setCursor", window_set_cursor, data);
    register(
        scope,
        object,
        "setCursorLocked",
        window_set_cursor_locked,
        data,
    );
    register(
        scope,
        object,
        "isCursorLocked",
        window_is_cursor_locked,
        data,
    );
    set_property(scope, native, "window", object.into());
}

fn window_size<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let state = host(&args);
    let size = state.window.inner_size();
    let logical = size.to_logical::<f64>(state.window.scale_factor());
    set_f64_array(
        scope,
        &mut rv,
        &[logical.width.round(), logical.height.round()],
    );
}

fn window_set_title<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let title = string_arg(scope, &args, 0);
    host(&args).window.set_title(&title);
}

fn window_set_fullscreen<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    use winit::window::Fullscreen;
    let state = host(&args);
    let enabled = bool_arg(scope, &args, 0);
    state.fullscreen.set(enabled);
    state.window.set_fullscreen(if enabled {
        Some(Fullscreen::Borderless(None))
    } else {
        None
    });
}

fn window_is_fullscreen<'s, 'i>(_scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    rv.set_bool(host(&args).fullscreen.get());
}

fn window_set_cursor<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let cursor = string_arg(scope, &args, 0);
    host(&args).set_cursor(&cursor);
}

fn window_set_cursor_locked<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let locked = bool_arg(scope, &args, 0);
    host(&args).set_cursor_locked(locked);
}

fn window_is_cursor_locked<'s, 'i>(_scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    rv.set_bool(host(&args).cursor_locked.get());
}

fn install_fs(scope: &PinScope<'_, '_>, native: Local<v8::Object>, data: Local<Value>) {
    let object = v8::Object::new(scope);
    register(scope, object, "size", fs_size, data);
    register(scope, object, "exists", fs_exists, data);
    register(scope, object, "read", fs_read, data);
    register(scope, object, "download", fs_download, data);
    set_property(scope, native, "fs", object.into());
}

fn fs_size<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let path = string_arg(scope, &args, 0);
    rv.set_int32(
        host(&args)
            .assets
            .size(&path)
            .map_or(-1, |size| size as i32),
    );
}

fn fs_exists<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let path = string_arg(scope, &args, 0);
    rv.set_bool(host(&args).assets.exists(&path));
}

fn fs_read<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let path = string_arg(scope, &args, 0);
    let Some(bytes) = host(&args).assets.read(&path) else {
        return rv.set_bool(false);
    };
    let Some(()) = with_u8_mut(args.get(1), |out| {
        let length = bytes.len().min(out.len());
        out[..length].copy_from_slice(&bytes[..length]);
    }) else {
        return rv.set_bool(false);
    };
    rv.set_bool(true);
}

fn fs_download<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let name = string_arg(scope, &args, 0);
    let path = with_u8(args.get(1), |bytes| host(&args).save_download(&name, bytes)).flatten();
    match path {
        Some(path) => set_string(scope, &mut rv, path.to_string_lossy().as_ref()),
        None => set_string(scope, &mut rv, ""),
    }
}

fn install_storage(scope: &PinScope<'_, '_>, native: Local<v8::Object>, data: Local<Value>) {
    let object = v8::Object::new(scope);
    register(scope, object, "load", storage_load, data);
    register(scope, object, "save", storage_save, data);
    set_property(scope, native, "storage", object.into());
}

fn storage_load<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let data = host(&args).load_storage();
    set_string(scope, &mut rv, &data);
}

fn storage_save<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let data = string_arg(scope, &args, 0);
    host(&args).save_storage(&data);
}

fn install_image(scope: &PinScope<'_, '_>, native: Local<v8::Object>, data: Local<Value>) {
    let object = v8::Object::new(scope);
    register(scope, object, "load", image_load, data);
    register(scope, object, "decode", image_decode, data);
    register(scope, object, "width", image_width, data);
    register(scope, object, "height", image_height, data);
    set_property(scope, native, "image", object.into());
}

fn image_load<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let url = string_arg(scope, &args, 0);
    let id = host(&args)
        .assets
        .read(&url)
        .and_then(|bytes| host(&args).decode_image(&bytes));
    rv.set_int32(handle(id));
}

fn image_decode<'s, 'i>(_scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let id = with_u8(args.get(0), |bytes| host(&args).decode_image(bytes)).flatten();
    rv.set_int32(handle(id));
}

fn image_width<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let id = u32_arg(scope, &args, 0);
    let state = host(&args);
    rv.set_int32(
        state
            .images
            .borrow()
            .get(&id)
            .map_or(-1, |image| image.width as i32),
    );
}

fn image_height<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let id = u32_arg(scope, &args, 0);
    let state = host(&args);
    rv.set_int32(
        state
            .images
            .borrow()
            .get(&id)
            .map_or(-1, |image| image.height as i32),
    );
}

fn install_font(scope: &PinScope<'_, '_>, native: Local<v8::Object>, data: Local<Value>) {
    let object = v8::Object::new(scope);
    register(scope, object, "register", font_register, data);
    register(scope, object, "has", font_has, data);
    set_property(scope, native, "font", object.into());
}

fn font_register<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let family = string_arg(scope, &args, 0);
    let Some(()) = with_u8(args.get(1), |bytes| {
        host(&args)
            .canvas
            .borrow_mut()
            .register_font(&family, bytes.to_vec());
    }) else {
        return rv.set_bool(false);
    };
    rv.set_bool(true);
}

fn font_has<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let family = string_arg(scope, &args, 0);
    rv.set_bool(host(&args).canvas.borrow().has_font(&family));
}

fn install_canvas(scope: &PinScope<'_, '_>, native: Local<v8::Object>, data: Local<Value>) {
    let object = v8::Object::new(scope);
    register(scope, object, "create", canvas_create, data);
    register(scope, object, "resize", canvas_resize, data);
    register(scope, object, "destroy", canvas_destroy, data);
    register(scope, object, "createContext", canvas_create_context, data);
    register(scope, object, "setFillStyle", canvas_set_fill_style, data);
    register(
        scope,
        object,
        "setStrokeStyle",
        canvas_set_stroke_style,
        data,
    );
    register(scope, object, "setFont", canvas_set_font, data);
    register(scope, object, "setLineWidth", canvas_set_line_width, data);
    register(scope, object, "setLineJoin", canvas_set_line_join, data);
    register(
        scope,
        object,
        "setTextBaseline",
        canvas_set_text_baseline,
        data,
    );
    register(scope, object, "setTextAlign", canvas_set_text_align, data);
    register(scope, object, "clearRect", canvas_clear_rect, data);
    register(scope, object, "fillText", canvas_fill_text, data);
    register(scope, object, "strokeText", canvas_stroke_text, data);
    register(scope, object, "measureText", canvas_measure_text, data);
    register(scope, object, "drawImage", canvas_draw_image, data);
    register(scope, object, "putImageData", canvas_put_image_data, data);
    register(scope, object, "getImageData", canvas_get_image_data, data);
    register(scope, object, "pngLength", canvas_png_length, data);
    register(scope, object, "copyPng", canvas_copy_png, data);
    set_property(scope, native, "canvas", object.into());
}

fn canvas_create<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let width = u32_arg(scope, &args, 0);
    let height = u32_arg(scope, &args, 1);
    rv.set_int32(host(&args).canvas.borrow_mut().create_canvas(width, height) as i32);
}

fn canvas_resize<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let id = u32_arg(scope, &args, 0);
    let width = u32_arg(scope, &args, 1);
    let height = u32_arg(scope, &args, 2);
    host(&args)
        .canvas
        .borrow_mut()
        .resize_canvas(id, width, height);
}

fn canvas_destroy<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let id = u32_arg(scope, &args, 0);
    host(&args).canvas.borrow_mut().destroy_canvas(id);
}

fn canvas_create_context<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let id = u32_arg(scope, &args, 0);
    rv.set_int32(handle(host(&args).canvas.borrow_mut().create_context(id)));
}

macro_rules! canvas_string_setter {
    ($name:ident, $method:ident) => {
        fn $name<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
            let context = u32_arg(scope, &args, 0);
            let value = string_arg(scope, &args, 1);
            host(&args).canvas.borrow_mut().$method(context, &value);
        }
    };
}

canvas_string_setter!(canvas_set_fill_style, set_fill_style);
canvas_string_setter!(canvas_set_stroke_style, set_stroke_style);
canvas_string_setter!(canvas_set_font, set_font);
canvas_string_setter!(canvas_set_line_join, set_line_join);
canvas_string_setter!(canvas_set_text_baseline, set_text_baseline);
canvas_string_setter!(canvas_set_text_align, set_text_align);

fn canvas_set_line_width<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let context = u32_arg(scope, &args, 0);
    let value = f64_arg(scope, &args, 1) as f32;
    host(&args)
        .canvas
        .borrow_mut()
        .set_line_width(context, value);
}

fn canvas_clear_rect<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let context = u32_arg(scope, &args, 0);
    let x = f64_arg(scope, &args, 1) as f32;
    let y = f64_arg(scope, &args, 2) as f32;
    let width = f64_arg(scope, &args, 3) as f32;
    let height = f64_arg(scope, &args, 4) as f32;
    host(&args)
        .canvas
        .borrow_mut()
        .clear_rect(context, x, y, width, height);
}

fn canvas_fill_text<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let context = u32_arg(scope, &args, 0);
    let text = string_arg(scope, &args, 1);
    let x = f64_arg(scope, &args, 2) as f32;
    let y = f64_arg(scope, &args, 3) as f32;
    host(&args)
        .canvas
        .borrow_mut()
        .fill_text(context, &text, x, y);
}

fn canvas_stroke_text<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let context = u32_arg(scope, &args, 0);
    let text = string_arg(scope, &args, 1);
    let x = f64_arg(scope, &args, 2) as f32;
    let y = f64_arg(scope, &args, 3) as f32;
    host(&args)
        .canvas
        .borrow_mut()
        .stroke_text(context, &text, x, y);
}

fn canvas_measure_text<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let context = u32_arg(scope, &args, 0);
    let text = string_arg(scope, &args, 1);
    let metrics = host(&args).canvas.borrow().measure_text(context, &text);
    set_f64_array(
        scope,
        &mut rv,
        &[
            metrics.width as f64,
            metrics.ascent as f64,
            metrics.descent as f64,
        ],
    );
}

fn canvas_draw_image<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let state = host(&args);
    let context = u32_arg(scope, &args, 0);
    let kind = i32_arg(scope, &args, 1);
    let id = u32_arg(scope, &args, 2);
    let count = args.length() - 3;
    let mut values = [0.0f64; 8];
    for (index, value) in values.iter_mut().enumerate().take(count.max(0) as usize) {
        *value = f64_arg(scope, &args, 3 + index as i32);
    }
    let (sx, sy, sw, sh, dx, dy, dw, dh) = if count >= 8 {
        (
            values[0] as f32,
            values[1] as f32,
            values[2] as f32,
            values[3] as f32,
            values[4] as f32,
            values[5] as f32,
            values[6] as f32,
            values[7] as f32,
        )
    } else if count >= 2 {
        let (width, height) = source_size(state, kind, id);
        (
            0.0,
            0.0,
            width as f32,
            height as f32,
            values[0] as f32,
            values[1] as f32,
            width as f32,
            height as f32,
        )
    } else {
        return;
    };
    let canvas = state.canvas.clone();
    if kind == 0 {
        let images = state.images.borrow();
        let Some(image) = images.get(&id) else {
            return;
        };
        canvas.borrow_mut().draw_image(
            context,
            ImageSource {
                width: image.width,
                height: image.height,
                data: &image.pixels,
            },
            sx,
            sy,
            sw,
            sh,
            dx,
            dy,
            dw,
            dh,
        );
    } else {
        canvas
            .borrow_mut()
            .draw_image_from_canvas(context, id, sx, sy, sw, sh, dx, dy, dw, dh);
    }
}

fn canvas_put_image_data<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let context = u32_arg(scope, &args, 0);
    let x = i32_arg(scope, &args, 1);
    let y = i32_arg(scope, &args, 2);
    let width = u32_arg(scope, &args, 3);
    let height = u32_arg(scope, &args, 4);
    let state = host(&args);
    with_u8(args.get(5), |bytes| {
        state
            .canvas
            .borrow_mut()
            .put_image_data(context, x, y, width, height, bytes);
    });
}

fn canvas_get_image_data<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let context = u32_arg(scope, &args, 0);
    let x = i32_arg(scope, &args, 1);
    let y = i32_arg(scope, &args, 2);
    let width = u32_arg(scope, &args, 3);
    let height = u32_arg(scope, &args, 4);
    let data = host(&args)
        .canvas
        .borrow()
        .get_image_data(context, x, y, width, height);
    with_u8_mut(args.get(5), |out| {
        let length = data.len().min(out.len());
        out[..length].copy_from_slice(&data[..length]);
    });
}

fn canvas_png_length<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let canvas = u32_arg(scope, &args, 0);
    rv.set_int32(
        host(&args)
            .canvas
            .borrow()
            .to_png(canvas)
            .map_or(-1, |data| data.len() as i32),
    );
}

fn canvas_copy_png<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let canvas = u32_arg(scope, &args, 0);
    let Some(data) = host(&args).canvas.borrow().to_png(canvas) else {
        return rv.set_bool(false);
    };
    let Some(()) = with_u8_mut(args.get(1), |out| {
        let length = data.len().min(out.len());
        out[..length].copy_from_slice(&data[..length]);
    }) else {
        return rv.set_bool(false);
    };
    rv.set_bool(true);
}

fn install_net(scope: &PinScope<'_, '_>, native: Local<v8::Object>, data: Local<Value>) {
    let object = v8::Object::new(scope);
    register(scope, object, "fetch", net_fetch, data);
    register(scope, object, "openSocket", net_open_socket, data);
    register(scope, object, "sendText", net_send_text, data);
    register(scope, object, "sendBinary", net_send_binary, data);
    register(scope, object, "closeSocket", net_close_socket, data);
    register(scope, object, "readResponse", net_read_response, data);
    register(scope, object, "readMessage", net_read_message, data);
    set_property(scope, native, "net", object.into());
}

fn install_auth(scope: &PinScope<'_, '_>, native: Local<v8::Object>, data: Local<Value>) {
    let object = v8::Object::new(scope);
    register(scope, object, "open", auth_open, data);
    set_property(scope, native, "auth", object.into());
}

fn auth_open<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let state = host(&args);
    let provider_url = string_arg(scope, &args, 0);
    let server = match crate::auth::AuthServer::start(&provider_url) {
        Ok(server) => server,
        Err(error) => {
            log::warn!("auth relay: {error}");
            return rv.set_bool(false);
        }
    };
    if let Err(error) = crate::auth::open_browser(server.relay_url()) {
        log::warn!("auth browser: {error}");
        return rv.set_bool(false);
    }
    *state.auth.borrow_mut() = Some(server);
    rv.set_bool(true);
}

fn net_fetch<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let state = host(&args);
    let id = u32_arg(scope, &args, 0);
    let method = string_arg(scope, &args, 1);
    let url = string_arg(scope, &args, 2);
    let headers = string_arg(scope, &args, 3);
    let body = with_u8(args.get(4), <[u8]>::to_vec).unwrap_or_default();
    let Some(network) = state.network.as_ref() else {
        return rv.set_bool(false);
    };
    rv.set_bool(network.fetch(kaplay_native_net::HttpRequest {
        id,
        method,
        url,
        headers: parse_headers(&headers),
        body,
    }));
}

fn net_open_socket<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let state = host(&args);
    let id = u32_arg(scope, &args, 0);
    let url = string_arg(scope, &args, 1);
    let Some(network) = state.network.as_ref() else {
        return rv.set_bool(false);
    };
    network.open_socket(id, url);
    rv.set_bool(true);
}

fn net_send_text<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let state = host(&args);
    let id = u32_arg(scope, &args, 0);
    let text = string_arg(scope, &args, 1);
    if let Some(network) = state.network.as_ref() {
        network.send_text(id, text);
    }
}

fn net_send_binary<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let state = host(&args);
    let id = u32_arg(scope, &args, 0);
    with_u8(args.get(1), |data| {
        if let Some(network) = state.network.as_ref() {
            network.send_binary(id, data.to_vec());
        }
    });
}

fn net_close_socket<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let state = host(&args);
    let id = u32_arg(scope, &args, 0);
    let code = u32_arg(scope, &args, 1);
    let reason = string_arg(scope, &args, 2);
    if let Some(network) = state.network.as_ref() {
        network.close_socket(id, code as u16, reason);
    }
}

fn net_read_response<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let state = host(&args);
    let id = u32_arg(scope, &args, 0);
    let Some(response) = state.pending_response.borrow_mut().take() else {
        return rv.set_int32(-1);
    };
    if response.id != id {
        *state.pending_response.borrow_mut() = Some(response);
        return rv.set_int32(-1);
    }
    let Some(length) = with_u8_mut(args.get(1), |out| {
        let length = out.len().min(response.body.len());
        out[..length].copy_from_slice(&response.body[..length]);
        length as i32
    }) else {
        return rv.set_int32(-1);
    };
    rv.set_int32(length);
}

fn net_read_message<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let state = host(&args);
    let id = u32_arg(scope, &args, 0);
    let Some(message) = state.pending_message.borrow_mut().take() else {
        return rv.set_int32(-1);
    };
    if message.socket != id {
        *state.pending_message.borrow_mut() = Some(message);
        return rv.set_int32(-1);
    }
    let Some(length) = with_u8_mut(args.get(1), |out| {
        let length = out.len().min(message.data.len());
        out[..length].copy_from_slice(&message.data[..length]);
        length as i32
    }) else {
        return rv.set_int32(-1);
    };
    rv.set_int32(length);
}

fn install_video(scope: &PinScope<'_, '_>, native: Local<v8::Object>, data: Local<Value>) {
    let object = v8::Object::new(scope);
    register(scope, object, "open", video_open, data);
    register(scope, object, "openAsset", video_open_asset, data);
    register(scope, object, "close", video_close, data);
    register(scope, object, "ready", video_ready, data);
    set_property(scope, native, "video", object.into());
}

fn video_open<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let state = host(&args);
    let Some(info) = with_u8(args.get(0), |bytes| state.open_video(Arc::from(bytes))).flatten()
    else {
        return;
    };
    video_info(scope, &mut rv, &info);
}

fn video_open_asset<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let state = host(&args);
    let path = string_arg(scope, &args, 0);
    let Some(info) = state.open_video_asset(&path) else {
        return;
    };
    video_info(scope, &mut rv, &info);
}

fn video_info(scope: &mut PinScope<'_, '_>, rv: &mut Rv<'_>, info: &VideoInfo) {
    let object = v8::Object::new(scope);
    set_property(
        scope,
        object,
        "id",
        v8::Number::new(scope, f64::from(info.id)).into(),
    );
    set_property(
        scope,
        object,
        "width",
        v8::Number::new(scope, f64::from(info.width)).into(),
    );
    set_property(
        scope,
        object,
        "height",
        v8::Number::new(scope, f64::from(info.height)).into(),
    );
    set_property(
        scope,
        object,
        "duration",
        v8::Number::new(scope, info.duration).into(),
    );
    rv.set(object.into());
}

fn video_close<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let id = u32_arg(scope, &args, 0);
    host(&args).close_video(id);
}

fn video_ready<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let id = u32_arg(scope, &args, 0);
    let time = f64_arg(scope, &args, 1);
    let ready = host(&args).video_frame_ready(id, time);
    rv.set_bool(ready);
}

fn install_audio(scope: &PinScope<'_, '_>, native: Local<v8::Object>, data: Local<Value>) {
    let object = v8::Object::new(scope);
    register(scope, object, "available", audio_available, data);
    register(scope, object, "sampleRate", audio_sample_rate, data);
    register(scope, object, "currentTime", audio_current_time, data);
    register(scope, object, "setPaused", audio_set_paused, data);
    register(scope, object, "decode", audio_decode, data);
    register(scope, object, "decodeAsset", audio_decode_asset, data);
    register(scope, object, "uploadBuffer", audio_upload_buffer, data);
    register(scope, object, "bufferInfo", audio_buffer_info, data);
    register(scope, object, "readChannel", audio_read_channel, data);
    register(scope, object, "freeBuffer", audio_free_buffer, data);
    register(scope, object, "play", audio_play, data);
    register(scope, object, "setGain", audio_set_gain, data);
    register(scope, object, "setPan", audio_set_pan, data);
    register(scope, object, "setRate", audio_set_rate, data);
    register(scope, object, "setLoop", audio_set_loop, data);
    register(scope, object, "stop", audio_stop, data);
    register(scope, object, "stopAt", audio_stop_at, data);
    register(scope, object, "position", audio_position, data);
    register(scope, object, "duration", audio_duration, data);
    set_property(scope, native, "audio", object.into());
}

fn audio_available<'s, 'i>(_scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let state = host(&args);
    rv.set_bool(state.audio.as_ref().is_some_and(|audio| audio.has_device()));
}

fn audio_sample_rate<'s, 'i>(_scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let state = host(&args);
    rv.set_uint32(state.audio.as_ref().map_or(0, |audio| audio.sample_rate()));
}

fn audio_current_time<'s, 'i>(_scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let state = host(&args);
    rv.set_double(
        state
            .audio
            .as_ref()
            .map_or(0.0, |audio| audio.current_time()),
    );
}

fn audio_set_paused<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let paused = bool_arg(scope, &args, 0);
    if let Some(audio) = host(&args).audio.as_ref() {
        audio.set_paused(paused);
    }
}

fn audio_decode<'s, 'i>(_scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let state = host(&args);
    let request = u32_arg(_scope, &args, 0);
    with_u8(args.get(1), |data| {
        if let Some(audio) = state.audio.as_ref() {
            audio.decode_async(request, Arc::from(data));
        }
    });
}

fn audio_decode_asset<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let state = host(&args);
    let request = u32_arg(scope, &args, 0);
    let path = string_arg(scope, &args, 1);
    state.decode_audio_asset(request, &path);
}

fn audio_upload_buffer<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let state = host(&args);
    let sample_rate = u32_arg(scope, &args, 0);
    let channels = u32_arg(scope, &args, 1);
    let Some(handle) = with_f32(args.get(2), |samples| {
        state
            .audio
            .as_ref()
            .map(|audio| audio.upload_buffer(sample_rate, channels as u16, samples.to_vec()))
    })
    .flatten() else {
        return rv.set_int32(-1);
    };
    rv.set_int32(super::handle(Some(handle)));
}

fn audio_buffer_info<'s, 'i>(_scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let state = host(&args);
    let id = u32_arg(_scope, &args, 0);
    let Some(info) = state.audio.as_ref().and_then(|audio| audio.buffer_info(id)) else {
        return rv.set_bool(false);
    };
    let Some(()) = with_f32_mut(args.get(1), |out| {
        let values = [info.0 as f32, info.1 as f32, info.2 as f32];
        let length = out.len().min(values.len());
        out[..length].copy_from_slice(&values[..length]);
    }) else {
        return rv.set_bool(false);
    };
    rv.set_bool(true);
}

fn audio_read_channel<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let state = host(&args);
    let id = u32_arg(scope, &args, 0);
    let channel = u32_arg(scope, &args, 1);
    let Some(audio) = state.audio.as_ref() else {
        return rv.set_int32(-1);
    };
    let Some(count) = with_f32_mut(args.get(2), |out| {
        audio.read_channel(id, channel as usize, out) as i32
    }) else {
        return rv.set_int32(-1);
    };
    rv.set_int32(count);
}

fn audio_free_buffer<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let id = u32_arg(scope, &args, 0);
    if let Some(audio) = host(&args).audio.as_ref() {
        audio.free_buffer(id);
    }
}

fn audio_play<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let state = host(&args);
    let buffer = u32_arg(scope, &args, 0);
    let gain = f64_arg(scope, &args, 1) as f32;
    let pan = f64_arg(scope, &args, 2) as f32;
    let rate = f64_arg(scope, &args, 3);
    let looping = bool_arg(scope, &args, 4);
    let offset = f64_arg(scope, &args, 5);
    let start_at = f64_arg(scope, &args, 6);
    let Some(audio) = state.audio.as_ref() else {
        return rv.set_int32(-1);
    };
    let start_at = (start_at >= 0.0).then_some(start_at as u64);
    rv.set_int32(handle(
        audio.play_voice(buffer, gain, pan, rate, looping, offset, start_at),
    ));
}

fn audio_set_gain<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let voice = u32_arg(scope, &args, 0);
    let gain = f64_arg(scope, &args, 1) as f32;
    if let Some(audio) = host(&args).audio.as_ref() {
        audio.set_voice_gain(voice, gain);
    }
}

fn audio_set_pan<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let voice = u32_arg(scope, &args, 0);
    let pan = f64_arg(scope, &args, 1) as f32;
    if let Some(audio) = host(&args).audio.as_ref() {
        audio.set_voice_pan(voice, pan);
    }
}

fn audio_set_rate<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let voice = u32_arg(scope, &args, 0);
    let rate = f64_arg(scope, &args, 1);
    if let Some(audio) = host(&args).audio.as_ref() {
        audio.set_voice_rate(voice, rate);
    }
}

fn audio_set_loop<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let voice = u32_arg(scope, &args, 0);
    let looping = bool_arg(scope, &args, 1);
    if let Some(audio) = host(&args).audio.as_ref() {
        audio.set_voice_loop(voice, looping);
    }
}

fn audio_stop<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let voice = u32_arg(scope, &args, 0);
    if let Some(audio) = host(&args).audio.as_ref() {
        audio.stop_voice(voice);
    }
}

fn audio_stop_at<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let voice = u32_arg(scope, &args, 0);
    let frame = f64_arg(scope, &args, 1);
    if let Some(audio) = host(&args).audio.as_ref() {
        audio.stop_voice_at(voice, frame.max(0.0) as u64);
    }
}

fn audio_position<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let voice = u32_arg(scope, &args, 0);
    let state = host(&args);
    rv.set_double(
        state
            .audio
            .as_ref()
            .and_then(|audio| audio.voice_position(voice))
            .unwrap_or(-1.0),
    );
}

fn audio_duration<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let voice = u32_arg(scope, &args, 0);
    let state = host(&args);
    rv.set_double(
        state
            .audio
            .as_ref()
            .and_then(|audio| audio.voice_duration(voice))
            .unwrap_or(-1.0),
    );
}

fn install_steam(scope: &PinScope<'_, '_>, native: Local<v8::Object>, data: Local<Value>) {
    let object = v8::Object::new(scope);
    register(scope, object, "available", steam_available, data);
    register(scope, object, "playerName", steam_player_name, data);
    register(scope, object, "steamId", steam_id, data);
    register(
        scope,
        object,
        "unlockAchievement",
        steam_unlock_achievement,
        data,
    );
    register(
        scope,
        object,
        "isAchievementUnlocked",
        steam_is_achievement_unlocked,
        data,
    );
    register(scope, object, "storeStats", steam_store_stats, data);
    register(scope, object, "setStatInt", steam_set_stat_int, data);
    register(scope, object, "getStatInt", steam_get_stat_int, data);
    register(scope, object, "setStatFloat", steam_set_stat_float, data);
    register(scope, object, "getStatFloat", steam_get_stat_float, data);
    register(scope, object, "writeCloud", steam_write_cloud, data);
    register(scope, object, "readCloud", steam_read_cloud, data);
    register(scope, object, "cloudExists", steam_cloud_exists, data);
    register(scope, object, "deleteCloud", steam_delete_cloud, data);
    register(scope, object, "runCallbacks", steam_run_callbacks, data);
    set_property(scope, native, "steam", object.into());
}

fn install_updates(scope: &PinScope<'_, '_>, native: Local<v8::Object>, data: Local<Value>) {
    let object = v8::Object::new(scope);
    register(scope, object, "snapshot", updates_snapshot, data);
    register(scope, object, "apply", updates_apply, data);
    set_property(scope, native, "updates", object.into());
}

fn updates_snapshot<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let state = host(&args);
    set_string(scope, &mut rv, &state.updates.snapshot_json());
}

fn updates_apply<'s, 'i>(_scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    rv.set_bool(host(&args).updates.request_restart());
}

fn steam_available<'s, 'i>(_scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let state = host(&args);
    rv.set_bool(
        state
            .steam
            .borrow()
            .as_ref()
            .is_some_and(|api| api.is_initialized()),
    );
}

fn steam_player_name<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let state = host(&args);
    let name = state
        .steam
        .borrow()
        .as_ref()
        .and_then(|api| api.player_name());
    set_opt_string(scope, &mut rv, name);
}

fn steam_id<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let state = host(&args);
    let id = state
        .steam
        .borrow()
        .as_ref()
        .map(|api| api.steam_id().to_string())
        .unwrap_or_default();
    set_string(scope, &mut rv, &id);
}

fn steam_unlock_achievement<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let name = string_arg(scope, &args, 0);
    let state = host(&args);
    rv.set_bool(
        state
            .steam
            .borrow()
            .as_ref()
            .is_some_and(|api| api.unlock_achievement(&name)),
    );
}

fn steam_is_achievement_unlocked<'s, 'i>(
    scope: &mut PinScope<'s, 'i>,
    args: Args<'s>,
    mut rv: Rv<'s>,
) {
    let name = string_arg(scope, &args, 0);
    let state = host(&args);
    rv.set_bool(
        state
            .steam
            .borrow()
            .as_ref()
            .is_some_and(|api| api.is_achievement_unlocked(&name)),
    );
}

fn steam_store_stats<'s, 'i>(_scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let state = host(&args);
    rv.set_bool(
        state
            .steam
            .borrow()
            .as_ref()
            .is_some_and(|api| api.store_stats()),
    );
}

fn steam_set_stat_int<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let name = string_arg(scope, &args, 0);
    let value = i32_arg(scope, &args, 1);
    let state = host(&args);
    rv.set_bool(
        state
            .steam
            .borrow()
            .as_ref()
            .is_some_and(|api| api.set_stat_int(&name, value)),
    );
}

fn steam_get_stat_int<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let name = string_arg(scope, &args, 0);
    let state = host(&args);
    match state
        .steam
        .borrow()
        .as_ref()
        .and_then(|api| api.get_stat_int(&name))
    {
        Some(value) => rv.set_int32(value),
        None => rv.set_undefined(),
    }
}

fn steam_set_stat_float<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let name = string_arg(scope, &args, 0);
    let value = f64_arg(scope, &args, 1) as f32;
    let state = host(&args);
    rv.set_bool(
        state
            .steam
            .borrow()
            .as_ref()
            .is_some_and(|api| api.set_stat_float(&name, value)),
    );
}

fn steam_get_stat_float<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let name = string_arg(scope, &args, 0);
    let state = host(&args);
    match state
        .steam
        .borrow()
        .as_ref()
        .and_then(|api| api.get_stat_float(&name))
    {
        Some(value) => rv.set_double(f64::from(value)),
        None => rv.set_undefined(),
    }
}

fn steam_write_cloud<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let name = string_arg(scope, &args, 0);
    let text = string_arg(scope, &args, 1);
    let state = host(&args);
    rv.set_bool(
        state
            .steam
            .borrow()
            .as_ref()
            .is_some_and(|api| api.write_file(&name, text.as_bytes())),
    );
}

fn steam_read_cloud<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let name = string_arg(scope, &args, 0);
    let state = host(&args);
    let data = state
        .steam
        .borrow()
        .as_ref()
        .and_then(|api| api.read_file(&name))
        .map(|data| String::from_utf8_lossy(&data).into_owned());
    set_opt_string(scope, &mut rv, data);
}

fn steam_cloud_exists<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let name = string_arg(scope, &args, 0);
    let state = host(&args);
    rv.set_bool(
        state
            .steam
            .borrow()
            .as_ref()
            .is_some_and(|api| api.file_exists(&name)),
    );
}

fn steam_delete_cloud<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let name = string_arg(scope, &args, 0);
    let state = host(&args);
    rv.set_bool(
        state
            .steam
            .borrow()
            .as_ref()
            .is_some_and(|api| api.delete_file(&name)),
    );
}

fn steam_run_callbacks<'s, 'i>(_scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let state = host(&args);
    if let Some(api) = state.steam.borrow().as_ref() {
        api.run_callbacks();
    }
}

fn install_gl(scope: &PinScope<'_, '_>, native: Local<v8::Object>, data: Local<Value>) {
    let object = v8::Object::new(scope);
    register(
        scope,
        object,
        "drawingBufferWidth",
        gl_drawing_buffer_width,
        data,
    );
    register(
        scope,
        object,
        "drawingBufferHeight",
        gl_drawing_buffer_height,
        data,
    );
    register(
        scope,
        object,
        "resizeDrawingBuffer",
        gl_resize_drawing_buffer,
        data,
    );
    register(scope, object, "viewport", gl_viewport, data);
    register(scope, object, "clearColor", gl_clear_color, data);
    register(scope, object, "clear", gl_clear, data);
    register(scope, object, "enable", gl_enable, data);
    register(scope, object, "disable", gl_disable, data);
    register(
        scope,
        object,
        "blendFuncSeparate",
        gl_blend_func_separate,
        data,
    );
    register(scope, object, "stencilFunc", gl_stencil_func, data);
    register(scope, object, "stencilOp", gl_stencil_op, data);
    register(scope, object, "pixelStorei", gl_pixel_storei, data);
    register(scope, object, "getParameter", gl_get_parameter, data);
    register(scope, object, "activeTexture", gl_active_texture, data);
    register(scope, object, "createTexture", gl_create_texture, data);
    register(scope, object, "deleteTexture", gl_delete_texture, data);
    register(scope, object, "bindTexture", gl_bind_texture, data);
    register(scope, object, "texParameteri", gl_tex_parameteri, data);
    register(scope, object, "texImage2D", gl_tex_image_2d, data);
    register(scope, object, "texSubImage2D", gl_tex_sub_image_2d, data);
    register(
        scope,
        object,
        "texImage2DImage",
        gl_tex_image_2d_image,
        data,
    );
    register(
        scope,
        object,
        "texSubImage2DImage",
        gl_tex_sub_image_2d_image,
        data,
    );
    register(
        scope,
        object,
        "texImage2DCanvas",
        gl_tex_image_2d_canvas,
        data,
    );
    register(
        scope,
        object,
        "texImage2DVideo",
        gl_tex_image_2d_video,
        data,
    );
    register(
        scope,
        object,
        "texSubImage2DCanvas",
        gl_tex_sub_image_2d_canvas,
        data,
    );
    register(scope, object, "createBuffer", gl_create_buffer, data);
    register(scope, object, "deleteBuffer", gl_delete_buffer, data);
    register(scope, object, "bindBuffer", gl_bind_buffer, data);
    register(scope, object, "bufferDataSize", gl_buffer_data_size, data);
    register(scope, object, "bufferData", gl_buffer_data, data);
    register(scope, object, "bufferSubData", gl_buffer_sub_data, data);
    register(
        scope,
        object,
        "enableVertexAttribArray",
        gl_enable_vertex_attrib_array,
        data,
    );
    register(
        scope,
        object,
        "disableVertexAttribArray",
        gl_disable_vertex_attrib_array,
        data,
    );
    register(
        scope,
        object,
        "vertexAttribPointer",
        gl_vertex_attrib_pointer,
        data,
    );
    register(scope, object, "drawArrays", gl_draw_arrays, data);
    register(scope, object, "drawElements", gl_draw_elements, data);
    register(scope, object, "createShader", gl_create_shader, data);
    register(scope, object, "deleteShader", gl_delete_shader, data);
    register(scope, object, "shaderSource", gl_shader_source, data);
    register(scope, object, "compileShader", gl_compile_shader, data);
    register(
        scope,
        object,
        "getShaderInfoLog",
        gl_get_shader_info_log,
        data,
    );
    register(scope, object, "createProgram", gl_create_program, data);
    register(scope, object, "deleteProgram", gl_delete_program, data);
    register(scope, object, "attachShader", gl_attach_shader, data);
    register(
        scope,
        object,
        "bindAttribLocation",
        gl_bind_attrib_location,
        data,
    );
    register(scope, object, "linkProgram", gl_link_program, data);
    register(scope, object, "getLinkStatus", gl_get_link_status, data);
    register(
        scope,
        object,
        "getProgramInfoLog",
        gl_get_program_info_log,
        data,
    );
    register(scope, object, "useProgram", gl_use_program, data);
    register(
        scope,
        object,
        "getAttribLocation",
        gl_get_attrib_location,
        data,
    );
    register(
        scope,
        object,
        "getUniformLocation",
        gl_get_uniform_location,
        data,
    );
    register(scope, object, "uniform1f", gl_uniform_1f, data);
    register(scope, object, "uniform1i", gl_uniform_1i, data);
    register(scope, object, "uniform2f", gl_uniform_2f, data);
    register(scope, object, "uniform3f", gl_uniform_3f, data);
    register(scope, object, "uniform1fv", gl_uniform_1fv, data);
    register(scope, object, "uniform2fv", gl_uniform_2fv, data);
    register(scope, object, "uniform3fv", gl_uniform_3fv, data);
    register(
        scope,
        object,
        "uniformMatrix4fv",
        gl_uniform_matrix_4fv,
        data,
    );
    register(
        scope,
        object,
        "createFramebuffer",
        gl_create_framebuffer,
        data,
    );
    register(
        scope,
        object,
        "deleteFramebuffer",
        gl_delete_framebuffer,
        data,
    );
    register(scope, object, "bindFramebuffer", gl_bind_framebuffer, data);
    register(
        scope,
        object,
        "createRenderbuffer",
        gl_create_renderbuffer,
        data,
    );
    register(
        scope,
        object,
        "deleteRenderbuffer",
        gl_delete_renderbuffer,
        data,
    );
    register(
        scope,
        object,
        "bindRenderbuffer",
        gl_bind_renderbuffer,
        data,
    );
    register(
        scope,
        object,
        "renderbufferStorage",
        gl_renderbuffer_storage,
        data,
    );
    register(
        scope,
        object,
        "framebufferTexture2D",
        gl_framebuffer_texture_2d,
        data,
    );
    register(
        scope,
        object,
        "framebufferRenderbuffer",
        gl_framebuffer_renderbuffer,
        data,
    );
    register(
        scope,
        object,
        "checkFramebufferStatus",
        gl_check_framebuffer_status,
        data,
    );
    register(scope, object, "readPixels", gl_read_pixels, data);
    register(
        scope,
        object,
        "screenshotLength",
        gl_screenshot_length,
        data,
    );
    register(scope, object, "copyScreenshot", gl_copy_screenshot, data);
    set_property(scope, native, "gl", object.into());
}

fn gl_arg_i32(scope: &PinScope<'_, '_>, args: &Args<'_>, index: i32) -> i32 {
    i32_arg(scope, args, index)
}

fn gl_arg_u32(scope: &PinScope<'_, '_>, args: &Args<'_>, index: i32) -> u32 {
    u32_arg(scope, args, index)
}

fn gl_arg_f32(scope: &PinScope<'_, '_>, args: &Args<'_>, index: i32) -> f32 {
    f64_arg(scope, args, index) as f32
}

fn gl_drawing_buffer_width<'s, 'i>(_scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    rv.set_int32(host(&args).gl.borrow().drawing_buffer_size().0);
}

fn gl_drawing_buffer_height<'s, 'i>(_scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    rv.set_int32(host(&args).gl.borrow().drawing_buffer_size().1);
}

fn gl_resize_drawing_buffer<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let width = gl_arg_i32(scope, &args, 0);
    let height = gl_arg_i32(scope, &args, 1);
    host(&args)
        .gl
        .borrow_mut()
        .resize_drawing_buffer(width, height);
}

fn gl_viewport<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let x = gl_arg_i32(scope, &args, 0);
    let y = gl_arg_i32(scope, &args, 1);
    let width = gl_arg_i32(scope, &args, 2);
    let height = gl_arg_i32(scope, &args, 3);
    host(&args).gl.borrow_mut().viewport(x, y, width, height);
}

fn gl_clear_color<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let r = gl_arg_f32(scope, &args, 0);
    let g = gl_arg_f32(scope, &args, 1);
    let b = gl_arg_f32(scope, &args, 2);
    let a = gl_arg_f32(scope, &args, 3);
    host(&args).gl.borrow_mut().clear_color(r, g, b, a);
}

fn gl_clear<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let mask = gl_arg_u32(scope, &args, 0);
    host(&args).gl.borrow_mut().clear(mask);
}

fn gl_enable<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let capability = gl_arg_u32(scope, &args, 0);
    host(&args).gl.borrow_mut().enable(capability);
}

fn gl_disable<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let capability = gl_arg_u32(scope, &args, 0);
    host(&args).gl.borrow_mut().disable(capability);
}

fn gl_blend_func_separate<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let srgb = gl_arg_u32(scope, &args, 0);
    let drgb = gl_arg_u32(scope, &args, 1);
    let salpha = gl_arg_u32(scope, &args, 2);
    let dalpha = gl_arg_u32(scope, &args, 3);
    host(&args)
        .gl
        .borrow_mut()
        .blend_func_separate(srgb, drgb, salpha, dalpha);
}

fn gl_stencil_func<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let func = gl_arg_u32(scope, &args, 0);
    let reference = gl_arg_i32(scope, &args, 1);
    let mask = gl_arg_u32(scope, &args, 2);
    host(&args)
        .gl
        .borrow_mut()
        .stencil_func(func, reference, mask);
}

fn gl_stencil_op<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let fail = gl_arg_u32(scope, &args, 0);
    let zfail = gl_arg_u32(scope, &args, 1);
    let zpass = gl_arg_u32(scope, &args, 2);
    host(&args).gl.borrow_mut().stencil_op(fail, zfail, zpass);
}

fn gl_pixel_storei<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let parameter = gl_arg_u32(scope, &args, 0);
    let value = gl_arg_i32(scope, &args, 1);
    host(&args)
        .gl
        .borrow_mut()
        .pixel_store_i32(parameter, value);
}

fn gl_get_parameter<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let parameter = gl_arg_u32(scope, &args, 0);
    rv.set_int32(host(&args).gl.borrow_mut().get_parameter_i32(parameter));
}

fn gl_active_texture<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let unit = gl_arg_u32(scope, &args, 0);
    host(&args).gl.borrow_mut().active_texture(unit);
}

fn gl_create_texture<'s, 'i>(_scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    rv.set_int32(handle(host(&args).gl.borrow_mut().create_texture()));
}

fn gl_delete_texture<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let texture = gl_arg_u32(scope, &args, 0);
    host(&args).gl.borrow_mut().delete_texture(texture);
}

fn gl_bind_texture<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let target = gl_arg_u32(scope, &args, 0);
    let texture = gl_arg_i32(scope, &args, 1);
    host(&args)
        .gl
        .borrow_mut()
        .bind_texture(target, option_handle(texture));
}

fn gl_tex_parameteri<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let target = gl_arg_u32(scope, &args, 0);
    let parameter = gl_arg_u32(scope, &args, 1);
    let value = gl_arg_i32(scope, &args, 2);
    host(&args)
        .gl
        .borrow_mut()
        .tex_parameter_i32(target, parameter, value);
}

fn gl_tex_image_2d<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let state = host(&args);
    let target = gl_arg_u32(scope, &args, 0);
    let internal_format = gl_arg_i32(scope, &args, 1);
    let width = gl_arg_i32(scope, &args, 2);
    let height = gl_arg_i32(scope, &args, 3);
    let format = gl_arg_u32(scope, &args, 4);
    let ty = gl_arg_u32(scope, &args, 5);
    let owned = with_u8(args.get(6), <[u8]>::to_vec);
    let options = state.gl.borrow().upload_options();
    state.gl.borrow_mut().tex_image_2d(
        target,
        0,
        internal_format,
        width,
        height,
        0,
        format,
        ty,
        owned.as_deref(),
        options,
    );
}

fn gl_tex_sub_image_2d<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let state = host(&args);
    let target = gl_arg_u32(scope, &args, 0);
    let x = gl_arg_i32(scope, &args, 1);
    let y = gl_arg_i32(scope, &args, 2);
    let width = gl_arg_i32(scope, &args, 3);
    let format = gl_arg_u32(scope, &args, 4);
    let ty = gl_arg_u32(scope, &args, 5);
    let Some(bytes) = with_u8(args.get(6), <[u8]>::to_vec) else {
        return;
    };
    let height = if width > 0 {
        (bytes.len() as i32 / (width * 4)).max(1)
    } else {
        0
    };
    let options = state.gl.borrow().upload_options();
    state
        .gl
        .borrow_mut()
        .tex_sub_image_2d(target, 0, x, y, width, height, format, ty, &bytes, options);
}

fn gl_tex_image_2d_image<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let state = host(&args);
    let target = gl_arg_u32(scope, &args, 0);
    let internal_format = gl_arg_i32(scope, &args, 1);
    let format = gl_arg_u32(scope, &args, 2);
    let ty = gl_arg_u32(scope, &args, 3);
    let image = gl_arg_u32(scope, &args, 4);
    let images = state.images.borrow();
    let Some(image) = images.get(&image) else {
        return;
    };
    let options = state.gl.borrow().upload_options();
    state.gl.borrow_mut().tex_image_2d(
        target,
        0,
        internal_format,
        image.width as i32,
        image.height as i32,
        0,
        format,
        ty,
        Some(&image.pixels),
        options,
    );
}

fn gl_tex_sub_image_2d_image<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let state = host(&args);
    let target = gl_arg_u32(scope, &args, 0);
    let x = gl_arg_i32(scope, &args, 1);
    let y = gl_arg_i32(scope, &args, 2);
    let format = gl_arg_u32(scope, &args, 3);
    let ty = gl_arg_u32(scope, &args, 4);
    let image = gl_arg_u32(scope, &args, 5);
    let images = state.images.borrow();
    let Some(image) = images.get(&image) else {
        return;
    };
    let options = state.gl.borrow().upload_options();
    state.gl.borrow_mut().tex_sub_image_2d(
        target,
        0,
        x,
        y,
        image.width as i32,
        image.height as i32,
        format,
        ty,
        &image.pixels,
        options,
    );
}

fn gl_tex_image_2d_video<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let state = host(&args);
    let target = gl_arg_u32(scope, &args, 0);
    let internal_format = gl_arg_i32(scope, &args, 1);
    let format = gl_arg_u32(scope, &args, 2);
    let ty = gl_arg_u32(scope, &args, 3);
    let video = gl_arg_u32(scope, &args, 4);
    let time = f64_arg(scope, &args, 5);
    state.upload_video_frame(video, time, target, internal_format, format, ty);
}

fn gl_tex_image_2d_canvas<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let state = host(&args);
    let target = gl_arg_u32(scope, &args, 0);
    let internal_format = gl_arg_i32(scope, &args, 1);
    let format = gl_arg_u32(scope, &args, 2);
    let ty = gl_arg_u32(scope, &args, 3);
    let canvas = gl_arg_u32(scope, &args, 4);
    let canvases = state.canvas.borrow();
    let Some((width, height)) = canvases.canvas_size(canvas) else {
        return;
    };
    let Some(bytes) = canvases.canvas_bytes(canvas) else {
        return;
    };
    let options = UploadOptions {
        flip_y: state.gl.borrow().upload_options().flip_y,
        premultiply: false,
    };
    state.gl.borrow_mut().tex_image_2d(
        target,
        0,
        internal_format,
        width as i32,
        height as i32,
        0,
        format,
        ty,
        Some(bytes),
        options,
    );
}

fn gl_tex_sub_image_2d_canvas<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let state = host(&args);
    let target = gl_arg_u32(scope, &args, 0);
    let x = gl_arg_i32(scope, &args, 1);
    let y = gl_arg_i32(scope, &args, 2);
    let format = gl_arg_u32(scope, &args, 3);
    let ty = gl_arg_u32(scope, &args, 4);
    let canvas = gl_arg_u32(scope, &args, 5);
    let canvases = state.canvas.borrow();
    let Some((width, height)) = canvases.canvas_size(canvas) else {
        return;
    };
    let Some(bytes) = canvases.canvas_bytes(canvas) else {
        return;
    };
    let options = UploadOptions {
        flip_y: state.gl.borrow().upload_options().flip_y,
        premultiply: false,
    };
    state.gl.borrow_mut().tex_sub_image_2d(
        target,
        0,
        x,
        y,
        width as i32,
        height as i32,
        format,
        ty,
        bytes,
        options,
    );
}

fn gl_create_buffer<'s, 'i>(_scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    rv.set_int32(handle(host(&args).gl.borrow_mut().create_buffer()));
}

fn gl_delete_buffer<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let buffer = gl_arg_u32(scope, &args, 0);
    host(&args).gl.borrow_mut().delete_buffer(buffer);
}

fn gl_bind_buffer<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let target = gl_arg_u32(scope, &args, 0);
    let buffer = gl_arg_i32(scope, &args, 1);
    host(&args)
        .gl
        .borrow_mut()
        .bind_buffer(target, option_handle(buffer));
}

fn gl_buffer_data_size<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let target = gl_arg_u32(scope, &args, 0);
    let size = gl_arg_i32(scope, &args, 1);
    let usage = gl_arg_u32(scope, &args, 2);
    host(&args)
        .gl
        .borrow_mut()
        .buffer_data_size(target, size, usage);
}

fn gl_buffer_data<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let state = host(&args);
    let target = gl_arg_u32(scope, &args, 0);
    let usage = gl_arg_u32(scope, &args, 2);
    with_u8(args.get(1), |bytes| {
        state
            .gl
            .borrow_mut()
            .buffer_data_bytes(target, bytes, usage);
    });
}

fn gl_buffer_sub_data<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let state = host(&args);
    let target = gl_arg_u32(scope, &args, 0);
    let offset = gl_arg_i32(scope, &args, 1);
    with_u8(args.get(2), |bytes| {
        state.gl.borrow_mut().buffer_sub_data(target, offset, bytes);
    });
}

fn gl_enable_vertex_attrib_array<'s, 'i>(
    scope: &mut PinScope<'s, 'i>,
    args: Args<'s>,
    _rv: Rv<'s>,
) {
    let index = gl_arg_u32(scope, &args, 0);
    host(&args)
        .gl
        .borrow_mut()
        .enable_vertex_attrib_array(index);
}

fn gl_disable_vertex_attrib_array<'s, 'i>(
    scope: &mut PinScope<'s, 'i>,
    args: Args<'s>,
    _rv: Rv<'s>,
) {
    let index = gl_arg_u32(scope, &args, 0);
    host(&args)
        .gl
        .borrow_mut()
        .disable_vertex_attrib_array(index);
}

fn gl_vertex_attrib_pointer<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let index = gl_arg_u32(scope, &args, 0);
    let size = gl_arg_i32(scope, &args, 1);
    let ty = gl_arg_u32(scope, &args, 2);
    let normalized = bool_arg(scope, &args, 3);
    let stride = gl_arg_i32(scope, &args, 4);
    let offset = gl_arg_i32(scope, &args, 5);
    host(&args)
        .gl
        .borrow_mut()
        .vertex_attrib_pointer(index, size, ty, normalized, stride, offset);
}

fn gl_draw_arrays<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let mode = gl_arg_u32(scope, &args, 0);
    let first = gl_arg_i32(scope, &args, 1);
    let count = gl_arg_i32(scope, &args, 2);
    host(&args).gl.borrow_mut().draw_arrays(mode, first, count);
}

fn gl_draw_elements<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let mode = gl_arg_u32(scope, &args, 0);
    let count = gl_arg_i32(scope, &args, 1);
    let ty = gl_arg_u32(scope, &args, 2);
    let offset = gl_arg_i32(scope, &args, 3);
    host(&args)
        .gl
        .borrow_mut()
        .draw_elements(mode, count, ty, offset);
}

fn gl_create_shader<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let kind = gl_arg_u32(scope, &args, 0);
    rv.set_int32(handle(host(&args).gl.borrow_mut().create_shader(kind)));
}

fn gl_delete_shader<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let shader = gl_arg_u32(scope, &args, 0);
    host(&args).gl.borrow_mut().delete_shader(shader);
}

fn gl_shader_source<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let shader = gl_arg_u32(scope, &args, 0);
    let source = string_arg(scope, &args, 1);
    rv.set_bool(
        host(&args)
            .gl
            .borrow_mut()
            .shader_source(shader, &source)
            .is_ok(),
    );
}

fn gl_compile_shader<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let shader = gl_arg_u32(scope, &args, 0);
    host(&args).gl.borrow_mut().compile_shader(shader);
}

fn gl_get_shader_info_log<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let shader = gl_arg_u32(scope, &args, 0);
    let log = host(&args).gl.borrow_mut().get_shader_info_log(shader);
    set_string(scope, &mut rv, &log);
}

fn gl_create_program<'s, 'i>(_scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    rv.set_int32(handle(host(&args).gl.borrow_mut().create_program()));
}

fn gl_delete_program<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let program = gl_arg_u32(scope, &args, 0);
    host(&args).gl.borrow_mut().delete_program(program);
}

fn gl_attach_shader<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let program = gl_arg_u32(scope, &args, 0);
    let shader = gl_arg_u32(scope, &args, 1);
    host(&args).gl.borrow_mut().attach_shader(program, shader);
}

fn gl_bind_attrib_location<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let program = gl_arg_u32(scope, &args, 0);
    let index = gl_arg_u32(scope, &args, 1);
    let name = string_arg(scope, &args, 2);
    host(&args)
        .gl
        .borrow_mut()
        .bind_attrib_location(program, index, &name);
}

fn gl_link_program<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let program = gl_arg_u32(scope, &args, 0);
    host(&args).gl.borrow_mut().link_program(program);
}

fn gl_get_link_status<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let program = gl_arg_u32(scope, &args, 0);
    rv.set_bool(host(&args).gl.borrow_mut().get_link_status(program));
}

fn gl_get_program_info_log<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let program = gl_arg_u32(scope, &args, 0);
    let log = host(&args).gl.borrow_mut().get_program_info_log(program);
    set_string(scope, &mut rv, &log);
}

fn gl_use_program<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let program = gl_arg_i32(scope, &args, 0);
    host(&args)
        .gl
        .borrow_mut()
        .use_program(option_handle(program));
}

fn gl_get_attrib_location<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let program = gl_arg_u32(scope, &args, 0);
    let name = string_arg(scope, &args, 1);
    rv.set_int32(
        host(&args)
            .gl
            .borrow_mut()
            .get_attrib_location(program, &name),
    );
}

fn gl_get_uniform_location<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let program = gl_arg_u32(scope, &args, 0);
    let name = string_arg(scope, &args, 1);
    rv.set_int32(
        host(&args)
            .gl
            .borrow_mut()
            .get_uniform_location(program, &name),
    );
}

fn gl_uniform_1f<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let location = gl_arg_i32(scope, &args, 0);
    let value = gl_arg_f32(scope, &args, 1);
    host(&args).gl.borrow_mut().uniform_1f(location, value);
}

fn gl_uniform_1i<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let location = gl_arg_i32(scope, &args, 0);
    let value = gl_arg_i32(scope, &args, 1);
    host(&args).gl.borrow_mut().uniform_1i(location, value);
}

fn gl_uniform_2f<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let location = gl_arg_i32(scope, &args, 0);
    let x = gl_arg_f32(scope, &args, 1);
    let y = gl_arg_f32(scope, &args, 2);
    host(&args).gl.borrow_mut().uniform_2f(location, x, y);
}

fn gl_uniform_3f<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let location = gl_arg_i32(scope, &args, 0);
    let x = gl_arg_f32(scope, &args, 1);
    let y = gl_arg_f32(scope, &args, 2);
    let z = gl_arg_f32(scope, &args, 3);
    host(&args).gl.borrow_mut().uniform_3f(location, x, y, z);
}

fn gl_uniform_1fv<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let state = host(&args);
    let location = gl_arg_i32(scope, &args, 0);
    with_f32(args.get(1), |values| {
        state.gl.borrow_mut().uniform_1fv(location, values);
    });
}

fn gl_uniform_2fv<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let state = host(&args);
    let location = gl_arg_i32(scope, &args, 0);
    with_f32(args.get(1), |values| {
        state.gl.borrow_mut().uniform_2fv(location, values);
    });
}

fn gl_uniform_3fv<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let state = host(&args);
    let location = gl_arg_i32(scope, &args, 0);
    with_f32(args.get(1), |values| {
        state.gl.borrow_mut().uniform_3fv(location, values);
    });
}

fn gl_uniform_matrix_4fv<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let state = host(&args);
    let location = gl_arg_i32(scope, &args, 0);
    with_f32(args.get(1), |values| {
        state.gl.borrow_mut().uniform_matrix_4fv(location, values);
    });
}

fn gl_create_framebuffer<'s, 'i>(_scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    rv.set_int32(handle(host(&args).gl.borrow_mut().create_framebuffer()));
}

fn gl_delete_framebuffer<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let framebuffer = gl_arg_u32(scope, &args, 0);
    host(&args).gl.borrow_mut().delete_framebuffer(framebuffer);
}

fn gl_bind_framebuffer<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let target = gl_arg_u32(scope, &args, 0);
    let framebuffer = gl_arg_i32(scope, &args, 1);
    host(&args)
        .gl
        .borrow_mut()
        .bind_framebuffer(target, option_handle(framebuffer));
}

fn gl_create_renderbuffer<'s, 'i>(_scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    rv.set_int32(handle(host(&args).gl.borrow_mut().create_renderbuffer()));
}

fn gl_delete_renderbuffer<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let renderbuffer = gl_arg_u32(scope, &args, 0);
    host(&args)
        .gl
        .borrow_mut()
        .delete_renderbuffer(renderbuffer);
}

fn gl_bind_renderbuffer<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let target = gl_arg_u32(scope, &args, 0);
    let renderbuffer = gl_arg_i32(scope, &args, 1);
    host(&args)
        .gl
        .borrow_mut()
        .bind_renderbuffer(target, option_handle(renderbuffer));
}

fn gl_renderbuffer_storage<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let target = gl_arg_u32(scope, &args, 0);
    let internal_format = gl_arg_u32(scope, &args, 1);
    let width = gl_arg_i32(scope, &args, 2);
    let height = gl_arg_i32(scope, &args, 3);
    host(&args)
        .gl
        .borrow_mut()
        .renderbuffer_storage(target, internal_format, width, height);
}

fn gl_framebuffer_texture_2d<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let target = gl_arg_u32(scope, &args, 0);
    let attachment = gl_arg_u32(scope, &args, 1);
    let texture_target = gl_arg_u32(scope, &args, 2);
    let texture = gl_arg_i32(scope, &args, 3);
    let level = gl_arg_i32(scope, &args, 4);
    host(&args).gl.borrow_mut().framebuffer_texture_2d(
        target,
        attachment,
        texture_target,
        option_handle(texture),
        level,
    );
}

fn gl_framebuffer_renderbuffer<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let target = gl_arg_u32(scope, &args, 0);
    let attachment = gl_arg_u32(scope, &args, 1);
    let renderbuffer_target = gl_arg_u32(scope, &args, 2);
    let renderbuffer = gl_arg_i32(scope, &args, 3);
    host(&args).gl.borrow_mut().framebuffer_renderbuffer(
        target,
        attachment,
        renderbuffer_target,
        option_handle(renderbuffer),
    );
}

fn gl_check_framebuffer_status<'s, 'i>(
    scope: &mut PinScope<'s, 'i>,
    args: Args<'s>,
    mut rv: Rv<'s>,
) {
    let target = gl_arg_u32(scope, &args, 0);
    rv.set_uint32(host(&args).gl.borrow_mut().check_framebuffer_status(target));
}

fn gl_read_pixels<'s, 'i>(scope: &mut PinScope<'s, 'i>, args: Args<'s>, _rv: Rv<'s>) {
    let state = host(&args);
    let x = gl_arg_i32(scope, &args, 0);
    let y = gl_arg_i32(scope, &args, 1);
    let width = gl_arg_i32(scope, &args, 2);
    let height = gl_arg_i32(scope, &args, 3);
    with_u8_mut(args.get(4), |out| {
        state.gl.borrow_mut().read_pixels(x, y, width, height, out);
    });
}

fn gl_screenshot_length<'s, 'i>(_scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let (width, height, pixels) = host(&args).gl.borrow_mut().read_default_pixels_top_down();
    rv.set_int32(
        crate::screenshots::encode_bytes(&pixels, width, height)
            .map_or(-1, |data| data.len() as i32),
    );
}

fn gl_copy_screenshot<'s, 'i>(_scope: &mut PinScope<'s, 'i>, args: Args<'s>, mut rv: Rv<'s>) {
    let (width, height, pixels) = host(&args).gl.borrow_mut().read_default_pixels_top_down();
    let Some(encoded) = crate::screenshots::encode_bytes(&pixels, width, height) else {
        return rv.set_bool(false);
    };
    let Some(()) = with_u8_mut(args.get(0), |out| {
        let length = encoded.len().min(out.len());
        out[..length].copy_from_slice(&encoded[..length]);
    }) else {
        return rv.set_bool(false);
    };
    rv.set_bool(true);
}
