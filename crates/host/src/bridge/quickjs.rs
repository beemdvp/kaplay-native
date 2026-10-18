use std::rc::Rc;
use std::sync::Arc;

use kaplay_native_canvas2d::ImageSource;
use kaplay_native_gfx::pixels::UploadOptions;
use rquickjs::function::Rest;
use rquickjs::{Ctx, Function, Object, TypedArray};

use super::{handle, option_handle, parse_headers, source_size};
use crate::state::{HostState, VideoInfo};

fn slice_u8<'a, 'js>(array: &'a TypedArray<'js, u8>) -> Option<&'a [u8]> {
    let raw = array.as_raw()?;
    Some(unsafe { std::slice::from_raw_parts(raw.as_ptr() as *const u8, raw.len()) })
}

fn slice_u8_mut<'a, 'js>(array: &'a mut TypedArray<'js, u8>) -> Option<&'a mut [u8]> {
    let raw = array.as_raw()?;
    Some(unsafe { std::slice::from_raw_parts_mut(raw.as_ptr() as *mut u8, raw.len()) })
}

fn slice_f32<'a, 'js>(array: &'a TypedArray<'js, f32>) -> Option<&'a [f32]> {
    let raw = array.as_raw()?;
    Some(unsafe { std::slice::from_raw_parts(raw.as_ptr() as *const f32, raw.len() / 4) })
}

fn slice_f32_mut<'a, 'js>(array: &'a mut TypedArray<'js, f32>) -> Option<&'a mut [f32]> {
    let raw = array.as_raw()?;
    Some(unsafe { std::slice::from_raw_parts_mut(raw.as_ptr() as *mut f32, raw.len() / 4) })
}

pub fn install<'js>(ctx: &Ctx<'js>, state: Rc<HostState>, shim: &str) -> rquickjs::Result<()> {
    let native = Object::new(ctx.clone())?;
    install_env(ctx, &native, &state)?;
    install_window(ctx, &native, &state)?;
    install_fs(ctx, &native, &state)?;
    install_storage(ctx, &native, &state)?;
    install_image(ctx, &native, &state)?;
    install_font(ctx, &native, &state)?;
    install_canvas(ctx, &native, &state)?;
    install_gl(ctx, &native, &state)?;
    install_net(ctx, &native, &state)?;
    install_auth(ctx, &native, &state)?;
    install_audio(ctx, &native, &state)?;
    install_video(ctx, &native, &state)?;
    install_gamepad(ctx, &native, &state)?;
    install_steam(ctx, &native, &state)?;
    install_updates(ctx, &native, &state)?;
    ctx.globals().set("__kapNative", native)?;
    ctx.eval::<(), _>(shim)?;
    Ok(())
}

fn install_env<'js>(
    ctx: &Ctx<'js>,
    native: &Object<'js>,
    state: &Rc<HostState>,
) -> rquickjs::Result<()> {
    let object = Object::new(ctx.clone())?;
    object.set("hostVersion", state.host_version)?;
    object.set("platform", state.platform)?;
    object.set("gameName", state.game_name.clone())?;
    object.set("networkAllowed", state.allow_network)?;
    object.set("startUrl", state.start_url.clone())?;
    {
        let state = state.clone();
        object.set(
            "now",
            Function::new(ctx.clone(), move || -> f64 { state.now_ms() })?,
        )?;
    }
    {
        object.set(
            "log",
            Function::new(
                ctx.clone(),
                move |level: String, message: String| match level.as_str() {
                    "error" => log::error!("{message}"),
                    "warn" => log::warn!("{message}"),
                    _ => log::info!("{message}"),
                },
            )?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "quit",
            Function::new(ctx.clone(), move || {
                state.quit.set(true);
            })?,
        )?;
    }
    native.set("env", object)?;
    Ok(())
}

fn install_window<'js>(
    ctx: &Ctx<'js>,
    native: &Object<'js>,
    state: &Rc<HostState>,
) -> rquickjs::Result<()> {
    let object = Object::new(ctx.clone())?;
    {
        let state = state.clone();
        object.set(
            "size",
            Function::new(ctx.clone(), move || -> Vec<i32> {
                let size = state.window.inner_size();
                let logical = size.to_logical::<f64>(state.window.scale_factor());
                vec![logical.width.round() as i32, logical.height.round() as i32]
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "setTitle",
            Function::new(ctx.clone(), move |title: String| {
                state.window.set_title(&title);
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "setFullscreen",
            Function::new(ctx.clone(), move |enabled: bool| {
                use winit::window::Fullscreen;
                state.fullscreen.set(enabled);
                state.window.set_fullscreen(if enabled {
                    Some(Fullscreen::Borderless(None))
                } else {
                    None
                });
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "isFullscreen",
            Function::new(ctx.clone(), move || -> bool { state.fullscreen.get() })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "setCursor",
            Function::new(ctx.clone(), move |cursor: String| {
                state.set_cursor(&cursor);
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "setCursorLocked",
            Function::new(ctx.clone(), move |locked: bool| {
                state.set_cursor_locked(locked);
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "isCursorLocked",
            Function::new(ctx.clone(), move || -> bool { state.cursor_locked.get() })?,
        )?;
    }
    native.set("window", object)?;
    Ok(())
}

fn install_fs<'js>(
    ctx: &Ctx<'js>,
    native: &Object<'js>,
    state: &Rc<HostState>,
) -> rquickjs::Result<()> {
    let object = Object::new(ctx.clone())?;
    {
        let state = state.clone();
        object.set(
            "size",
            Function::new(ctx.clone(), move |path: String| -> i32 {
                state.assets.size(&path).map_or(-1, |size| size as i32)
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "exists",
            Function::new(ctx.clone(), move |path: String| -> bool {
                state.assets.exists(&path)
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "read",
            Function::new(
                ctx.clone(),
                move |path: String, mut out: TypedArray<u8>| -> bool {
                    let Some(bytes) = state.assets.read(&path) else {
                        return false;
                    };
                    let Some(out) = slice_u8_mut(&mut out) else {
                        return false;
                    };
                    let length = bytes.len().min(out.len());
                    out[..length].copy_from_slice(&bytes[..length]);
                    true
                },
            )?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "download",
            Function::new(
                ctx.clone(),
                move |name: String, data: TypedArray<u8>| -> String {
                    let Some(bytes) = slice_u8(&data) else {
                        return String::new();
                    };
                    state
                        .save_download(&name, bytes)
                        .map(|path| path.to_string_lossy().into_owned())
                        .unwrap_or_default()
                },
            )?,
        )?;
    }
    native.set("fs", object)?;
    Ok(())
}

fn install_storage<'js>(
    ctx: &Ctx<'js>,
    native: &Object<'js>,
    state: &Rc<HostState>,
) -> rquickjs::Result<()> {
    let object = Object::new(ctx.clone())?;
    {
        let state = state.clone();
        object.set(
            "load",
            Function::new(ctx.clone(), move || -> String { state.load_storage() })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "save",
            Function::new(ctx.clone(), move |data: String| {
                state.save_storage(&data);
            })?,
        )?;
    }
    native.set("storage", object)?;
    Ok(())
}

fn install_image<'js>(
    ctx: &Ctx<'js>,
    native: &Object<'js>,
    state: &Rc<HostState>,
) -> rquickjs::Result<()> {
    let object = Object::new(ctx.clone())?;
    {
        let state = state.clone();
        object.set(
            "load",
            Function::new(ctx.clone(), move |url: String| -> i32 {
                let Some(bytes) = state.assets.read(&url) else {
                    return -1;
                };
                handle(state.decode_image(&bytes))
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "decode",
            Function::new(ctx.clone(), move |data: TypedArray<u8>| -> i32 {
                let Some(bytes) = slice_u8(&data) else {
                    return -1;
                };
                handle(state.decode_image(bytes))
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "width",
            Function::new(ctx.clone(), move |id: u32| -> i32 {
                state
                    .images
                    .borrow()
                    .get(&id)
                    .map_or(-1, |image| image.width as i32)
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "height",
            Function::new(ctx.clone(), move |id: u32| -> i32 {
                state
                    .images
                    .borrow()
                    .get(&id)
                    .map_or(-1, |image| image.height as i32)
            })?,
        )?;
    }
    native.set("image", object)?;
    Ok(())
}

fn install_font<'js>(
    ctx: &Ctx<'js>,
    native: &Object<'js>,
    state: &Rc<HostState>,
) -> rquickjs::Result<()> {
    let object = Object::new(ctx.clone())?;
    {
        let state = state.clone();
        object.set(
            "register",
            Function::new(
                ctx.clone(),
                move |family: String, data: TypedArray<u8>| -> bool {
                    let Some(bytes) = slice_u8(&data) else {
                        return false;
                    };
                    state
                        .canvas
                        .borrow_mut()
                        .register_font(&family, bytes.to_vec());
                    true
                },
            )?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "has",
            Function::new(ctx.clone(), move |family: String| -> bool {
                state.canvas.borrow().has_font(&family)
            })?,
        )?;
    }
    native.set("font", object)?;
    Ok(())
}

fn install_canvas<'js>(
    ctx: &Ctx<'js>,
    native: &Object<'js>,
    state: &Rc<HostState>,
) -> rquickjs::Result<()> {
    let object = Object::new(ctx.clone())?;
    {
        let state = state.clone();
        object.set(
            "create",
            Function::new(ctx.clone(), move |width: u32, height: u32| -> i32 {
                state.canvas.borrow_mut().create_canvas(width, height) as i32
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "resize",
            Function::new(ctx.clone(), move |id: u32, width: u32, height: u32| {
                state.canvas.borrow_mut().resize_canvas(id, width, height);
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "destroy",
            Function::new(ctx.clone(), move |id: u32| {
                state.canvas.borrow_mut().destroy_canvas(id);
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "createContext",
            Function::new(ctx.clone(), move |id: u32| -> i32 {
                handle(state.canvas.borrow_mut().create_context(id))
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "setFillStyle",
            Function::new(ctx.clone(), move |context: u32, value: String| {
                state.canvas.borrow_mut().set_fill_style(context, &value);
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "setStrokeStyle",
            Function::new(ctx.clone(), move |context: u32, value: String| {
                state.canvas.borrow_mut().set_stroke_style(context, &value);
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "setFont",
            Function::new(ctx.clone(), move |context: u32, value: String| {
                state.canvas.borrow_mut().set_font(context, &value);
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "setLineWidth",
            Function::new(ctx.clone(), move |context: u32, value: f32| {
                state.canvas.borrow_mut().set_line_width(context, value);
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "setLineJoin",
            Function::new(ctx.clone(), move |context: u32, value: String| {
                state.canvas.borrow_mut().set_line_join(context, &value);
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "setTextBaseline",
            Function::new(ctx.clone(), move |context: u32, value: String| {
                state.canvas.borrow_mut().set_text_baseline(context, &value);
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "setTextAlign",
            Function::new(ctx.clone(), move |context: u32, value: String| {
                state.canvas.borrow_mut().set_text_align(context, &value);
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "clearRect",
            Function::new(
                ctx.clone(),
                move |context: u32, x: f32, y: f32, width: f32, height: f32| {
                    state
                        .canvas
                        .borrow_mut()
                        .clear_rect(context, x, y, width, height);
                },
            )?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "fillText",
            Function::new(
                ctx.clone(),
                move |context: u32, text: String, x: f32, y: f32| {
                    state.canvas.borrow_mut().fill_text(context, &text, x, y);
                },
            )?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "strokeText",
            Function::new(
                ctx.clone(),
                move |context: u32, text: String, x: f32, y: f32| {
                    state.canvas.borrow_mut().stroke_text(context, &text, x, y);
                },
            )?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "measureText",
            Function::new(ctx.clone(), move |context: u32, text: String| -> Vec<f64> {
                let metrics = state.canvas.borrow().measure_text(context, &text);
                vec![
                    metrics.width as f64,
                    metrics.ascent as f64,
                    metrics.descent as f64,
                ]
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "drawImage",
            Function::new(
                ctx.clone(),
                move |context: u32, kind: i32, id: u32, args: Rest<f64>| {
                    let values: Vec<f64> = args.iter().copied().collect();
                    let (sx, sy, sw, sh, dx, dy, dw, dh) = if values.len() >= 8 {
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
                    } else if values.len() >= 2 {
                        let (width, height) = source_size(&state, kind, id);
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
                },
            )?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "putImageData",
            Function::new(
                ctx.clone(),
                move |context: u32,
                      x: i32,
                      y: i32,
                      width: u32,
                      height: u32,
                      data: TypedArray<u8>| {
                    let Some(bytes) = slice_u8(&data) else {
                        return;
                    };
                    state
                        .canvas
                        .borrow_mut()
                        .put_image_data(context, x, y, width, height, bytes);
                },
            )?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "getImageData",
            Function::new(
                ctx.clone(),
                move |context: u32,
                      x: i32,
                      y: i32,
                      width: u32,
                      height: u32,
                      mut out: TypedArray<u8>| {
                    let data = state
                        .canvas
                        .borrow()
                        .get_image_data(context, x, y, width, height);
                    let Some(out) = slice_u8_mut(&mut out) else {
                        return;
                    };
                    let length = data.len().min(out.len());
                    out[..length].copy_from_slice(&data[..length]);
                },
            )?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "pngLength",
            Function::new(ctx.clone(), move |canvas: u32| -> i32 {
                state
                    .canvas
                    .borrow()
                    .to_png(canvas)
                    .map_or(-1, |data| data.len() as i32)
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "copyPng",
            Function::new(
                ctx.clone(),
                move |canvas: u32, mut out: TypedArray<u8>| -> bool {
                    let Some(data) = state.canvas.borrow().to_png(canvas) else {
                        return false;
                    };
                    let Some(out) = slice_u8_mut(&mut out) else {
                        return false;
                    };
                    let length = data.len().min(out.len());
                    out[..length].copy_from_slice(&data[..length]);
                    true
                },
            )?,
        )?;
    }
    native.set("canvas", object)?;
    Ok(())
}

fn install_net<'js>(
    ctx: &Ctx<'js>,
    native: &Object<'js>,
    state: &Rc<HostState>,
) -> rquickjs::Result<()> {
    let object = Object::new(ctx.clone())?;
    {
        let state = state.clone();
        object.set(
            "fetch",
            Function::new(
                ctx.clone(),
                move |id: u32,
                      method: String,
                      url: String,
                      headers: String,
                      body: Option<TypedArray<u8>>|
                      -> bool {
                    let Some(network) = state.network.as_ref() else {
                        return false;
                    };
                    let body = body
                        .as_ref()
                        .and_then(slice_u8)
                        .map(<[u8]>::to_vec)
                        .unwrap_or_default();
                    network.fetch(kaplay_native_net::HttpRequest {
                        id,
                        method,
                        url,
                        headers: parse_headers(&headers),
                        body,
                    })
                },
            )?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "openSocket",
            Function::new(ctx.clone(), move |id: u32, url: String| -> bool {
                let Some(network) = state.network.as_ref() else {
                    return false;
                };
                network.open_socket(id, url);
                true
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "sendText",
            Function::new(ctx.clone(), move |id: u32, text: String| {
                if let Some(network) = state.network.as_ref() {
                    network.send_text(id, text);
                }
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "sendBinary",
            Function::new(ctx.clone(), move |id: u32, data: TypedArray<u8>| {
                let Some(network) = state.network.as_ref() else {
                    return;
                };
                let Some(data) = slice_u8(&data) else {
                    return;
                };
                network.send_binary(id, data.to_vec());
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "closeSocket",
            Function::new(ctx.clone(), move |id: u32, code: u32, reason: String| {
                if let Some(network) = state.network.as_ref() {
                    network.close_socket(id, code as u16, reason);
                }
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "readResponse",
            Function::new(
                ctx.clone(),
                move |id: u32, mut out: TypedArray<u8>| -> i32 {
                    let Some(response) = state.pending_response.borrow_mut().take() else {
                        return -1;
                    };
                    if response.id != id {
                        *state.pending_response.borrow_mut() = Some(response);
                        return -1;
                    }
                    let Some(out) = slice_u8_mut(&mut out) else {
                        return -1;
                    };
                    let length = out.len().min(response.body.len());
                    out[..length].copy_from_slice(&response.body[..length]);
                    length as i32
                },
            )?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "readMessage",
            Function::new(
                ctx.clone(),
                move |id: u32, mut out: TypedArray<u8>| -> i32 {
                    let Some(message) = state.pending_message.borrow_mut().take() else {
                        return -1;
                    };
                    if message.socket != id {
                        *state.pending_message.borrow_mut() = Some(message);
                        return -1;
                    }
                    let Some(out) = slice_u8_mut(&mut out) else {
                        return -1;
                    };
                    let length = out.len().min(message.data.len());
                    out[..length].copy_from_slice(&message.data[..length]);
                    length as i32
                },
            )?,
        )?;
    }
    native.set("net", object)?;
    Ok(())
}

fn install_auth<'js>(
    ctx: &Ctx<'js>,
    native: &Object<'js>,
    state: &Rc<HostState>,
) -> rquickjs::Result<()> {
    let object = Object::new(ctx.clone())?;
    let state = state.clone();
    object.set(
        "open",
        Function::new(ctx.clone(), move |provider_url: String| -> bool {
            let server = match crate::auth::AuthServer::start(&provider_url) {
                Ok(server) => server,
                Err(error) => {
                    log::warn!("auth relay: {error}");
                    return false;
                }
            };
            if let Err(error) = crate::auth::open_browser(server.relay_url()) {
                log::warn!("auth browser: {error}");
                return false;
            }
            *state.auth.borrow_mut() = Some(server);
            true
        })?,
    )?;
    native.set("auth", object)?;
    Ok(())
}

fn install_audio<'js>(
    ctx: &Ctx<'js>,
    native: &Object<'js>,
    state: &Rc<HostState>,
) -> rquickjs::Result<()> {
    let object = Object::new(ctx.clone())?;
    {
        let state = state.clone();
        object.set(
            "available",
            Function::new(ctx.clone(), move || -> bool {
                state.audio.as_ref().is_some_and(|audio| audio.has_device())
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "sampleRate",
            Function::new(ctx.clone(), move || -> u32 {
                state.audio.as_ref().map_or(0, |audio| audio.sample_rate())
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "currentTime",
            Function::new(ctx.clone(), move || -> f64 {
                state
                    .audio
                    .as_ref()
                    .map_or(0.0, |audio| audio.current_time())
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "setPaused",
            Function::new(ctx.clone(), move |paused: bool| {
                if let Some(audio) = state.audio.as_ref() {
                    audio.set_paused(paused);
                }
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "decode",
            Function::new(ctx.clone(), move |request: u32, data: TypedArray<u8>| {
                let Some(audio) = state.audio.as_ref() else {
                    return;
                };
                let Some(data) = slice_u8(&data) else {
                    return;
                };
                audio.decode_async(request, Arc::from(data));
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "decodeAsset",
            Function::new(ctx.clone(), move |request: u32, path: String| {
                state.decode_audio_asset(request, &path);
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "uploadBuffer",
            Function::new(
                ctx.clone(),
                move |sample_rate: u32, channels: u32, data: TypedArray<f32>| -> i32 {
                    let Some(audio) = state.audio.as_ref() else {
                        return -1;
                    };
                    let Some(samples) = slice_f32(&data) else {
                        return -1;
                    };
                    handle(Some(audio.upload_buffer(
                        sample_rate,
                        channels as u16,
                        samples.to_vec(),
                    )))
                },
            )?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "bufferInfo",
            Function::new(
                ctx.clone(),
                move |id: u32, mut out: TypedArray<f32>| -> bool {
                    let Some(audio) = state.audio.as_ref() else {
                        return false;
                    };
                    let Some((sample_rate, channels, frames)) = audio.buffer_info(id) else {
                        return false;
                    };
                    let Some(out) = slice_f32_mut(&mut out) else {
                        return false;
                    };
                    let values = [sample_rate as f32, channels as f32, frames as f32];
                    let length = out.len().min(values.len());
                    out[..length].copy_from_slice(&values[..length]);
                    true
                },
            )?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "readChannel",
            Function::new(
                ctx.clone(),
                move |id: u32, channel: u32, mut out: TypedArray<f32>| -> i32 {
                    let Some(audio) = state.audio.as_ref() else {
                        return -1;
                    };
                    let Some(out) = slice_f32_mut(&mut out) else {
                        return -1;
                    };
                    audio.read_channel(id, channel as usize, out) as i32
                },
            )?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "freeBuffer",
            Function::new(ctx.clone(), move |id: u32| {
                if let Some(audio) = state.audio.as_ref() {
                    audio.free_buffer(id);
                }
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "play",
            Function::new(
                ctx.clone(),
                move |buffer: u32,
                      gain: f64,
                      pan: f64,
                      rate: f64,
                      looping: bool,
                      offset: f64,
                      start_at: f64|
                      -> i32 {
                    let Some(audio) = state.audio.as_ref() else {
                        return -1;
                    };
                    let start_at = (start_at >= 0.0).then_some(start_at as u64);
                    handle(audio.play_voice(
                        buffer,
                        gain as f32,
                        pan as f32,
                        rate,
                        looping,
                        offset,
                        start_at,
                    ))
                },
            )?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "setGain",
            Function::new(ctx.clone(), move |voice: u32, gain: f64| {
                if let Some(audio) = state.audio.as_ref() {
                    audio.set_voice_gain(voice, gain as f32);
                }
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "setPan",
            Function::new(ctx.clone(), move |voice: u32, pan: f64| {
                if let Some(audio) = state.audio.as_ref() {
                    audio.set_voice_pan(voice, pan as f32);
                }
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "setRate",
            Function::new(ctx.clone(), move |voice: u32, rate: f64| {
                if let Some(audio) = state.audio.as_ref() {
                    audio.set_voice_rate(voice, rate);
                }
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "setLoop",
            Function::new(ctx.clone(), move |voice: u32, looping: bool| {
                if let Some(audio) = state.audio.as_ref() {
                    audio.set_voice_loop(voice, looping);
                }
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "stop",
            Function::new(ctx.clone(), move |voice: u32| {
                if let Some(audio) = state.audio.as_ref() {
                    audio.stop_voice(voice);
                }
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "stopAt",
            Function::new(ctx.clone(), move |voice: u32, frame: f64| {
                if let Some(audio) = state.audio.as_ref() {
                    audio.stop_voice_at(voice, frame.max(0.0) as u64);
                }
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "position",
            Function::new(ctx.clone(), move |voice: u32| -> f64 {
                state
                    .audio
                    .as_ref()
                    .and_then(|audio| audio.voice_position(voice))
                    .unwrap_or(-1.0)
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "duration",
            Function::new(ctx.clone(), move |voice: u32| -> f64 {
                state
                    .audio
                    .as_ref()
                    .and_then(|audio| audio.voice_duration(voice))
                    .unwrap_or(-1.0)
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "analyser",
            Function::new(ctx.clone(), move |mut out: TypedArray<f32>| -> u32 {
                let Some(audio) = state.audio.as_ref() else {
                    return 0;
                };
                let Some(out) = slice_f32_mut(&mut out) else {
                    return 0;
                };
                audio.read_analyser(out) as u32
            })?,
        )?;
    }
    native.set("audio", object)?;
    Ok(())
}

fn install_video<'js>(
    ctx: &Ctx<'js>,
    native: &Object<'js>,
    state: &Rc<HostState>,
) -> rquickjs::Result<()> {
    let object = Object::new(ctx.clone())?;
    {
        let state = state.clone();
        object.set(
            "open",
            Function::new(
                ctx.clone(),
                move |context: Ctx<'js>, bytes: TypedArray<u8>| -> Option<Object<'js>> {
                    let data = slice_u8(&bytes)?;
                    let info = state.open_video(Arc::from(data))?;
                    video_info(context, &info)
                },
            )?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "openAsset",
            Function::new(
                ctx.clone(),
                move |context: Ctx<'js>, path: String| -> Option<Object<'js>> {
                    let info = state.open_video_asset(&path)?;
                    video_info(context, &info)
                },
            )?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "close",
            Function::new(ctx.clone(), move |id: u32| {
                state.close_video(id);
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "ready",
            Function::new(ctx.clone(), move |id: u32, time: f64| {
                state.video_frame_ready(id, time)
            })?,
        )?;
    }
    native.set("video", object)?;
    Ok(())
}

fn video_info<'js>(context: Ctx<'js>, info: &VideoInfo) -> Option<Object<'js>> {
    let value = Object::new(context).ok()?;
    value.set("id", info.id).ok()?;
    value.set("width", info.width).ok()?;
    value.set("height", info.height).ok()?;
    value.set("duration", info.duration).ok()?;
    Some(value)
}

fn install_gamepad<'js>(
    ctx: &Ctx<'js>,
    native: &Object<'js>,
    state: &Rc<HostState>,
) -> rquickjs::Result<()> {
    let object = Object::new(ctx.clone())?;
    let state = state.clone();
    object.set(
        "list",
        Function::new(ctx.clone(), move || -> String { state.gamepad_json() })?,
    )?;
    native.set("gamepad", object)?;
    Ok(())
}

fn install_steam<'js>(
    ctx: &Ctx<'js>,
    native: &Object<'js>,
    state: &Rc<HostState>,
) -> rquickjs::Result<()> {
    let object = Object::new(ctx.clone())?;
    {
        let state = state.clone();
        object.set(
            "available",
            Function::new(ctx.clone(), move || -> bool {
                state
                    .steam
                    .borrow()
                    .as_ref()
                    .is_some_and(|api| api.is_initialized())
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "playerName",
            Function::new(ctx.clone(), move || -> Option<String> {
                state
                    .steam
                    .borrow()
                    .as_ref()
                    .and_then(|api| api.player_name())
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "steamId",
            Function::new(ctx.clone(), move || -> String {
                state
                    .steam
                    .borrow()
                    .as_ref()
                    .map(|api| api.steam_id().to_string())
                    .unwrap_or_default()
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "unlockAchievement",
            Function::new(ctx.clone(), move |name: String| -> bool {
                state
                    .steam
                    .borrow()
                    .as_ref()
                    .is_some_and(|api| api.unlock_achievement(&name))
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "isAchievementUnlocked",
            Function::new(ctx.clone(), move |name: String| -> bool {
                state
                    .steam
                    .borrow()
                    .as_ref()
                    .is_some_and(|api| api.is_achievement_unlocked(&name))
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "storeStats",
            Function::new(ctx.clone(), move || -> bool {
                state
                    .steam
                    .borrow()
                    .as_ref()
                    .is_some_and(|api| api.store_stats())
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "setStatInt",
            Function::new(ctx.clone(), move |name: String, value: i32| -> bool {
                state
                    .steam
                    .borrow()
                    .as_ref()
                    .is_some_and(|api| api.set_stat_int(&name, value))
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "getStatInt",
            Function::new(ctx.clone(), move |name: String| -> Option<i32> {
                state
                    .steam
                    .borrow()
                    .as_ref()
                    .and_then(|api| api.get_stat_int(&name))
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "setStatFloat",
            Function::new(ctx.clone(), move |name: String, value: f32| -> bool {
                state
                    .steam
                    .borrow()
                    .as_ref()
                    .is_some_and(|api| api.set_stat_float(&name, value))
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "getStatFloat",
            Function::new(ctx.clone(), move |name: String| -> Option<f32> {
                state
                    .steam
                    .borrow()
                    .as_ref()
                    .and_then(|api| api.get_stat_float(&name))
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "writeCloud",
            Function::new(ctx.clone(), move |name: String, text: String| -> bool {
                state
                    .steam
                    .borrow()
                    .as_ref()
                    .is_some_and(|api| api.write_file(&name, text.as_bytes()))
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "readCloud",
            Function::new(ctx.clone(), move |name: String| -> Option<String> {
                state
                    .steam
                    .borrow()
                    .as_ref()
                    .and_then(|api| api.read_file(&name))
                    .map(|data| String::from_utf8_lossy(&data).into_owned())
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "cloudExists",
            Function::new(ctx.clone(), move |name: String| -> bool {
                state
                    .steam
                    .borrow()
                    .as_ref()
                    .is_some_and(|api| api.file_exists(&name))
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "deleteCloud",
            Function::new(ctx.clone(), move |name: String| -> bool {
                state
                    .steam
                    .borrow()
                    .as_ref()
                    .is_some_and(|api| api.delete_file(&name))
            })?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "runCallbacks",
            Function::new(ctx.clone(), move || {
                if let Some(api) = state.steam.borrow().as_ref() {
                    api.run_callbacks();
                }
            })?,
        )?;
    }
    native.set("steam", object)?;
    Ok(())
}

fn install_updates<'js>(
    ctx: &Ctx<'js>,
    native: &Object<'js>,
    state: &Rc<HostState>,
) -> rquickjs::Result<()> {
    let object = Object::new(ctx.clone())?;
    {
        let state = state.clone();
        object.set(
            "snapshot",
            Function::new(ctx.clone(), move || state.updates.snapshot_json())?,
        )?;
    }
    {
        let state = state.clone();
        object.set(
            "apply",
            Function::new(ctx.clone(), move || state.updates.request_restart())?,
        )?;
    }
    native.set("updates", object)?;
    Ok(())
}

fn install_gl<'js>(
    ctx: &Ctx<'js>,
    native: &Object<'js>,
    state: &Rc<HostState>,
) -> rquickjs::Result<()> {
    let object = Object::new(ctx.clone())?;
    macro_rules! register {
        ($name:literal, $closure:expr) => {
            object.set($name, Function::new(ctx.clone(), $closure)?)?
        };
    }

    {
        let state = state.clone();
        register!("drawingBufferWidth", move || -> i32 {
            state.gl.borrow().drawing_buffer_size().0
        });
    }
    {
        let state = state.clone();
        register!("drawingBufferHeight", move || -> i32 {
            state.gl.borrow().drawing_buffer_size().1
        });
    }
    {
        let state = state.clone();
        register!("resizeDrawingBuffer", move |width: i32, height: i32| {
            state.gl.borrow_mut().resize_drawing_buffer(width, height);
        });
    }
    {
        let state = state.clone();
        register!(
            "viewport",
            move |x: i32, y: i32, width: i32, height: i32| {
                state.gl.borrow_mut().viewport(x, y, width, height);
            }
        );
    }
    {
        let state = state.clone();
        register!("clearColor", move |r: f32, g: f32, b: f32, a: f32| {
            state.gl.borrow_mut().clear_color(r, g, b, a);
        });
    }
    {
        let state = state.clone();
        register!("clear", move |mask: u32| {
            state.gl.borrow_mut().clear(mask);
        });
    }
    {
        let state = state.clone();
        register!("enable", move |capability: u32| {
            state.gl.borrow_mut().enable(capability);
        });
    }
    {
        let state = state.clone();
        register!("disable", move |capability: u32| {
            state.gl.borrow_mut().disable(capability);
        });
    }
    {
        let state = state.clone();
        register!(
            "blendFuncSeparate",
            move |srgb: u32, drgb: u32, salpha: u32, dalpha: u32| {
                state
                    .gl
                    .borrow_mut()
                    .blend_func_separate(srgb, drgb, salpha, dalpha);
            }
        );
    }
    {
        let state = state.clone();
        register!(
            "stencilFunc",
            move |func: u32, reference: i32, mask: u32| {
                state.gl.borrow_mut().stencil_func(func, reference, mask);
            }
        );
    }
    {
        let state = state.clone();
        register!("stencilOp", move |fail: u32, zfail: u32, zpass: u32| {
            state.gl.borrow_mut().stencil_op(fail, zfail, zpass);
        });
    }
    {
        let state = state.clone();
        register!("pixelStorei", move |parameter: u32, value: i32| {
            state.gl.borrow_mut().pixel_store_i32(parameter, value);
        });
    }
    {
        let state = state.clone();
        register!("getParameter", move |parameter: u32| -> i32 {
            state.gl.borrow_mut().get_parameter_i32(parameter)
        });
    }
    {
        let state = state.clone();
        register!("activeTexture", move |unit: u32| {
            state.gl.borrow_mut().active_texture(unit);
        });
    }
    {
        let state = state.clone();
        register!("createTexture", move || -> i32 {
            handle(state.gl.borrow_mut().create_texture())
        });
    }
    {
        let state = state.clone();
        register!("deleteTexture", move |handle: u32| {
            state.gl.borrow_mut().delete_texture(handle);
        });
    }
    {
        let state = state.clone();
        register!("bindTexture", move |target: u32, handle: i32| {
            state
                .gl
                .borrow_mut()
                .bind_texture(target, option_handle(handle));
        });
    }
    {
        let state = state.clone();
        register!(
            "texParameteri",
            move |target: u32, parameter: u32, value: i32| {
                state
                    .gl
                    .borrow_mut()
                    .tex_parameter_i32(target, parameter, value);
            }
        );
    }
    {
        let state = state.clone();
        register!(
            "texImage2D",
            move |target: u32,
                  internal_format: i32,
                  width: i32,
                  height: i32,
                  format: u32,
                  ty: u32,
                  pixels: Option<TypedArray<u8>>| {
                let options = state.gl.borrow().upload_options();
                let source = pixels.as_ref().and_then(slice_u8);
                state.gl.borrow_mut().tex_image_2d(
                    target,
                    0,
                    internal_format,
                    width,
                    height,
                    0,
                    format,
                    ty,
                    source,
                    options,
                );
            }
        );
    }
    {
        let state = state.clone();
        register!(
            "texSubImage2D",
            move |target: u32,
                  x: i32,
                  y: i32,
                  width: i32,
                  format: u32,
                  ty: u32,
                  pixels: TypedArray<u8>| {
                let Some(bytes) = slice_u8(&pixels) else {
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
                    .tex_sub_image_2d(target, 0, x, y, width, height, format, ty, bytes, options);
            }
        );
    }
    {
        let state = state.clone();
        register!(
            "texImage2DImage",
            move |target: u32, internal_format: i32, format: u32, ty: u32, image: u32| {
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
        );
    }
    {
        let state = state.clone();
        register!(
            "texSubImage2DImage",
            move |target: u32, x: i32, y: i32, format: u32, ty: u32, image: u32| {
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
        );
    }
    {
        let state = state.clone();
        register!(
            "texImage2DCanvas",
            move |target: u32, internal_format: i32, format: u32, ty: u32, canvas: u32| {
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
        );
    }
    {
        let state = state.clone();
        register!(
            "texImage2DVideo",
            move |target: u32,
                  internal_format: i32,
                  format: u32,
                  ty: u32,
                  video: u32,
                  time: f64| {
                state.upload_video_frame(video, time, target, internal_format, format, ty);
            }
        );
    }
    {
        let state = state.clone();
        register!(
            "texSubImage2DCanvas",
            move |target: u32, x: i32, y: i32, format: u32, ty: u32, canvas: u32| {
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
        );
    }
    {
        let state = state.clone();
        register!("createBuffer", move || -> i32 {
            handle(state.gl.borrow_mut().create_buffer())
        });
    }
    {
        let state = state.clone();
        register!("deleteBuffer", move |handle: u32| {
            state.gl.borrow_mut().delete_buffer(handle);
        });
    }
    {
        let state = state.clone();
        register!("bindBuffer", move |target: u32, handle: i32| {
            state
                .gl
                .borrow_mut()
                .bind_buffer(target, option_handle(handle));
        });
    }
    {
        let state = state.clone();
        register!(
            "bufferDataSize",
            move |target: u32, size: i32, usage: u32| {
                state.gl.borrow_mut().buffer_data_size(target, size, usage);
            }
        );
    }
    {
        let state = state.clone();
        register!(
            "bufferData",
            move |target: u32, data: TypedArray<u8>, usage: u32| {
                if let Some(bytes) = slice_u8(&data) {
                    state
                        .gl
                        .borrow_mut()
                        .buffer_data_bytes(target, bytes, usage);
                }
            }
        );
    }
    {
        let state = state.clone();
        register!(
            "bufferSubData",
            move |target: u32, offset: i32, data: TypedArray<u8>| {
                if let Some(bytes) = slice_u8(&data) {
                    state.gl.borrow_mut().buffer_sub_data(target, offset, bytes);
                }
            }
        );
    }
    {
        let state = state.clone();
        register!("enableVertexAttribArray", move |index: u32| {
            state.gl.borrow_mut().enable_vertex_attrib_array(index);
        });
    }
    {
        let state = state.clone();
        register!("disableVertexAttribArray", move |index: u32| {
            state.gl.borrow_mut().disable_vertex_attrib_array(index);
        });
    }
    {
        let state = state.clone();
        register!(
            "vertexAttribPointer",
            move |index: u32, size: i32, ty: u32, normalized: bool, stride: i32, offset: i32| {
                state
                    .gl
                    .borrow_mut()
                    .vertex_attrib_pointer(index, size, ty, normalized, stride, offset);
            }
        );
    }
    {
        let state = state.clone();
        register!("drawArrays", move |mode: u32, first: i32, count: i32| {
            state.gl.borrow_mut().draw_arrays(mode, first, count);
        });
    }
    {
        let state = state.clone();
        register!(
            "drawElements",
            move |mode: u32, count: i32, ty: u32, offset: i32| {
                state.gl.borrow_mut().draw_elements(mode, count, ty, offset);
            }
        );
    }
    {
        let state = state.clone();
        register!("createShader", move |kind: u32| -> i32 {
            handle(state.gl.borrow_mut().create_shader(kind))
        });
    }
    {
        let state = state.clone();
        register!("deleteShader", move |handle: u32| {
            state.gl.borrow_mut().delete_shader(handle);
        });
    }
    {
        let state = state.clone();
        register!("shaderSource", move |handle: u32, source: String| -> bool {
            state.gl.borrow_mut().shader_source(handle, &source).is_ok()
        });
    }
    {
        let state = state.clone();
        register!("compileShader", move |handle: u32| {
            state.gl.borrow_mut().compile_shader(handle);
        });
    }
    {
        let state = state.clone();
        register!("getShaderInfoLog", move |handle: u32| -> String {
            state.gl.borrow_mut().get_shader_info_log(handle)
        });
    }
    {
        let state = state.clone();
        register!("createProgram", move || -> i32 {
            handle(state.gl.borrow_mut().create_program())
        });
    }
    {
        let state = state.clone();
        register!("deleteProgram", move |handle: u32| {
            state.gl.borrow_mut().delete_program(handle);
        });
    }
    {
        let state = state.clone();
        register!("attachShader", move |program: u32, shader: u32| {
            state.gl.borrow_mut().attach_shader(program, shader);
        });
    }
    {
        let state = state.clone();
        register!(
            "bindAttribLocation",
            move |program: u32, index: u32, name: String| {
                state
                    .gl
                    .borrow_mut()
                    .bind_attrib_location(program, index, &name);
            }
        );
    }
    {
        let state = state.clone();
        register!("linkProgram", move |handle: u32| {
            state.gl.borrow_mut().link_program(handle);
        });
    }
    {
        let state = state.clone();
        register!("getLinkStatus", move |handle: u32| -> bool {
            state.gl.borrow_mut().get_link_status(handle)
        });
    }
    {
        let state = state.clone();
        register!("getProgramInfoLog", move |handle: u32| -> String {
            state.gl.borrow_mut().get_program_info_log(handle)
        });
    }
    {
        let state = state.clone();
        register!("useProgram", move |handle: i32| {
            state.gl.borrow_mut().use_program(option_handle(handle));
        });
    }
    {
        let state = state.clone();
        register!("getAttribLocation", move |program: u32,
                                             name: String|
              -> i32 {
            state.gl.borrow_mut().get_attrib_location(program, &name)
        });
    }
    {
        let state = state.clone();
        register!("getUniformLocation", move |program: u32,
                                              name: String|
              -> i32 {
            state.gl.borrow_mut().get_uniform_location(program, &name)
        });
    }
    {
        let state = state.clone();
        register!("uniform1f", move |location: i32, value: f32| {
            state.gl.borrow_mut().uniform_1f(location, value);
        });
    }
    {
        let state = state.clone();
        register!("uniform1i", move |location: i32, value: i32| {
            state.gl.borrow_mut().uniform_1i(location, value);
        });
    }
    {
        let state = state.clone();
        register!("uniform2f", move |location: i32, x: f32, y: f32| {
            state.gl.borrow_mut().uniform_2f(location, x, y);
        });
    }
    {
        let state = state.clone();
        register!("uniform3f", move |location: i32, x: f32, y: f32, z: f32| {
            state.gl.borrow_mut().uniform_3f(location, x, y, z);
        });
    }
    {
        let state = state.clone();
        register!(
            "uniform1fv",
            move |location: i32, values: TypedArray<f32>| {
                if let Some(values) = slice_f32(&values) {
                    state.gl.borrow_mut().uniform_1fv(location, values);
                }
            }
        );
    }
    {
        let state = state.clone();
        register!(
            "uniform2fv",
            move |location: i32, values: TypedArray<f32>| {
                if let Some(values) = slice_f32(&values) {
                    state.gl.borrow_mut().uniform_2fv(location, values);
                }
            }
        );
    }
    {
        let state = state.clone();
        register!(
            "uniform3fv",
            move |location: i32, values: TypedArray<f32>| {
                if let Some(values) = slice_f32(&values) {
                    state.gl.borrow_mut().uniform_3fv(location, values);
                }
            }
        );
    }
    {
        let state = state.clone();
        register!(
            "uniformMatrix4fv",
            move |location: i32, values: TypedArray<f32>| {
                if let Some(values) = slice_f32(&values) {
                    state.gl.borrow_mut().uniform_matrix_4fv(location, values);
                }
            }
        );
    }
    {
        let state = state.clone();
        register!("createFramebuffer", move || -> i32 {
            handle(state.gl.borrow_mut().create_framebuffer())
        });
    }
    {
        let state = state.clone();
        register!("deleteFramebuffer", move |handle: u32| {
            state.gl.borrow_mut().delete_framebuffer(handle);
        });
    }
    {
        let state = state.clone();
        register!("bindFramebuffer", move |target: u32, handle: i32| {
            state
                .gl
                .borrow_mut()
                .bind_framebuffer(target, option_handle(handle));
        });
    }
    {
        let state = state.clone();
        register!("createRenderbuffer", move || -> i32 {
            handle(state.gl.borrow_mut().create_renderbuffer())
        });
    }
    {
        let state = state.clone();
        register!("deleteRenderbuffer", move |handle: u32| {
            state.gl.borrow_mut().delete_renderbuffer(handle);
        });
    }
    {
        let state = state.clone();
        register!("bindRenderbuffer", move |target: u32, handle: i32| {
            state
                .gl
                .borrow_mut()
                .bind_renderbuffer(target, option_handle(handle));
        });
    }
    {
        let state = state.clone();
        register!(
            "renderbufferStorage",
            move |target: u32, internal_format: u32, width: i32, height: i32| {
                state
                    .gl
                    .borrow_mut()
                    .renderbuffer_storage(target, internal_format, width, height);
            }
        );
    }
    {
        let state = state.clone();
        register!(
            "framebufferTexture2D",
            move |target: u32, attachment: u32, texture_target: u32, texture: i32, level: i32| {
                state.gl.borrow_mut().framebuffer_texture_2d(
                    target,
                    attachment,
                    texture_target,
                    option_handle(texture),
                    level,
                );
            }
        );
    }
    {
        let state = state.clone();
        register!(
            "framebufferRenderbuffer",
            move |target: u32, attachment: u32, renderbuffer_target: u32, renderbuffer: i32| {
                state.gl.borrow_mut().framebuffer_renderbuffer(
                    target,
                    attachment,
                    renderbuffer_target,
                    option_handle(renderbuffer),
                );
            }
        );
    }
    {
        let state = state.clone();
        register!("checkFramebufferStatus", move |target: u32| -> u32 {
            state.gl.borrow_mut().check_framebuffer_status(target)
        });
    }
    {
        let state = state.clone();
        register!(
            "readPixels",
            move |x: i32, y: i32, width: i32, height: i32, mut out: TypedArray<u8>| {
                let Some(out) = slice_u8_mut(&mut out) else {
                    return;
                };
                state.gl.borrow_mut().read_pixels(x, y, width, height, out);
            }
        );
    }
    {
        let state = state.clone();
        register!("screenshotLength", move || -> i32 {
            let (width, height, pixels) = state.gl.borrow_mut().read_default_pixels_top_down();
            crate::screenshots::encode_bytes(&pixels, width, height)
                .map_or(-1, |data| data.len() as i32)
        });
    }
    {
        let state = state.clone();
        register!("copyScreenshot", move |mut out: TypedArray<u8>| -> bool {
            let (width, height, pixels) = state.gl.borrow_mut().read_default_pixels_top_down();
            let Some(encoded) = crate::screenshots::encode_bytes(&pixels, width, height) else {
                return false;
            };
            let Some(out) = slice_u8_mut(&mut out) else {
                return false;
            };
            let length = encoded.len().min(out.len());
            out[..length].copy_from_slice(&encoded[..length]);
            true
        });
    }
    native.set("gl", object)?;
    Ok(())
}
