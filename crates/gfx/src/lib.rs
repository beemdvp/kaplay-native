use std::borrow::Cow;
use std::collections::HashMap;

use glow::{HasContext, NativeTexture};

pub mod pixels;
pub mod shader;

use pixels::UploadOptions;

pub const VERTEX_SHADER: u32 = 0x8b31;
pub const FRAGMENT_SHADER: u32 = 0x8b30;
pub const FRAMEBUFFER: u32 = 0x8d40;
pub const FRAMEBUFFER_COMPLETE: u32 = 0x8cd5;

const BLIT_VERT: &str = r#"attribute vec2 a_pos;varying vec2 v_uv;void main(){v_uv=a_pos*0.5+0.5;gl_Position=vec4(a_pos,0.0,1.0);}"#;
const BLIT_FRAG: &str =
    r#"varying vec2 v_uv;uniform sampler2D u_tex;void main(){gl_FragColor=texture2D(u_tex,v_uv);}"#;

struct DefaultFramebuffer {
    framebuffer: glow::Framebuffer,
    color: glow::Texture,
    depth_stencil: glow::Renderbuffer,
    width: i32,
    height: i32,
}

struct Blit {
    program: glow::Program,
    vbo: glow::Buffer,
    texture_location: Option<glow::UniformLocation>,
    position_location: u32,
}

pub struct GlState {
    gl: glow::Context,
    width: i32,
    height: i32,
    default_framebuffer: Option<DefaultFramebuffer>,
    blit: Option<Blit>,
    next_handle: u32,
    textures: HashMap<u32, glow::Texture>,
    buffers: HashMap<u32, glow::Buffer>,
    shaders: HashMap<u32, glow::Shader>,
    shader_types: HashMap<u32, u32>,
    programs: HashMap<u32, glow::Program>,
    framebuffers: HashMap<u32, glow::Framebuffer>,
    renderbuffers: HashMap<u32, glow::Renderbuffer>,
    uniform_locations: HashMap<u32, glow::UniformLocation>,
    uniform_cache: HashMap<u32, HashMap<String, i32>>,
    attrib_cache: HashMap<u32, HashMap<String, i32>>,
    shader_logs: HashMap<u32, String>,
    flip_y: bool,
    premultiply: bool,
    attribs: HashMap<u32, AttribPointer>,
    array_buffer: Option<u32>,
    profile: shader::Profile,
    vertex_array: Option<glow::VertexArray>,
}

#[derive(Clone, Copy, Default)]
struct AttribPointer {
    enabled: bool,
    buffer: Option<u32>,
    size: i32,
    ty: u32,
    normalized: bool,
    stride: i32,
    offset: i32,
}

fn detect_profile(gl: &glow::Context) -> shader::Profile {
    let version = unsafe { gl.get_parameter_string(glow::VERSION) };
    let mut parts = version
        .split(['.', ' '])
        .filter_map(|part| part.parse::<u32>().ok());
    let major = parts.next().unwrap_or(2);
    let minor = parts.next().unwrap_or(1);
    if (major, minor) >= (3, 2) {
        let mask = unsafe { gl.get_parameter_i32(glow::CONTEXT_PROFILE_MASK) } as u32;
        if mask & glow::CONTEXT_CORE_PROFILE_BIT != 0 {
            return shader::Profile::Core;
        }
    }
    shader::Profile::Compatibility
}

impl GlState {
    pub fn new(gl: glow::Context) -> Self {
        let profile = detect_profile(&gl);
        let mut state = Self {
            gl,
            width: 1,
            height: 1,
            default_framebuffer: None,
            blit: None,
            next_handle: 1,
            textures: HashMap::new(),
            buffers: HashMap::new(),
            shaders: HashMap::new(),
            shader_types: HashMap::new(),
            programs: HashMap::new(),
            framebuffers: HashMap::new(),
            renderbuffers: HashMap::new(),
            uniform_locations: HashMap::new(),
            uniform_cache: HashMap::new(),
            attrib_cache: HashMap::new(),
            shader_logs: HashMap::new(),
            flip_y: false,
            premultiply: false,
            attribs: HashMap::new(),
            array_buffer: None,
            profile,
            vertex_array: None,
        };
        if profile == shader::Profile::Core {
            state.vertex_array = unsafe { state.gl.create_vertex_array().ok() };
            if let Some(vertex_array) = state.vertex_array {
                unsafe { state.gl.bind_vertex_array(Some(vertex_array)) };
            } else {
                log::error!("failed to create the core profile vertex array");
            }
        }
        state.blit = state.create_blit();
        state
    }

    pub fn drawing_buffer_size(&self) -> (i32, i32) {
        (self.width, self.height)
    }

    pub fn resize_drawing_buffer(&mut self, width: i32, height: i32) {
        let width = width.max(1);
        let height = height.max(1);
        if width == self.width && height == self.height && self.default_framebuffer.is_some() {
            return;
        }
        self.width = width;
        self.height = height;
        self.rebuild_default_framebuffer();
    }

    fn create_blit(&mut self) -> Option<Blit> {
        unsafe {
            let gl = &self.gl;
            let program = gl.create_program().ok()?;
            let vert = gl.create_shader(VERTEX_SHADER).ok()?;
            let vertex_source = shader::translate(BLIT_VERT, shader::Stage::Vertex, self.profile)
                .map_err(|error| log::error!("blit vertex shader: {error}"))
                .ok()?;
            gl.shader_source(vert, &vertex_source);
            gl.compile_shader(vert);
            let frag = gl.create_shader(FRAGMENT_SHADER).ok()?;
            let fragment_source =
                shader::translate(BLIT_FRAG, shader::Stage::Fragment, self.profile)
                    .map_err(|error| log::error!("blit fragment shader: {error}"))
                    .ok()?;
            gl.shader_source(frag, &fragment_source);
            gl.compile_shader(frag);
            gl.attach_shader(program, vert);
            gl.attach_shader(program, frag);
            gl.bind_attrib_location(program, 0, "a_pos");
            gl.link_program(program);
            if gl.get_program_parameter_i32(program, glow::LINK_STATUS) == 0 {
                log::error!(
                    "blit program link failed: {}",
                    gl.get_program_info_log(program)
                );
                return None;
            }
            gl.delete_shader(vert);
            gl.delete_shader(frag);

            let vbo = gl.create_buffer().ok()?;
            let vertices: [f32; 8] = [-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0];
            let bytes = vertices
                .iter()
                .flat_map(|value| value.to_ne_bytes())
                .collect::<Vec<u8>>();
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &bytes, glow::STATIC_DRAW);

            Some(Blit {
                program,
                vbo,
                texture_location: gl.get_uniform_location(program, "u_tex"),
                position_location: 0,
            })
        }
    }

    fn rebuild_default_framebuffer(&mut self) {
        unsafe {
            let gl = &self.gl;
            if let Some(old) = self.default_framebuffer.take() {
                gl.delete_framebuffer(old.framebuffer);
                gl.delete_texture(old.color);
                gl.delete_renderbuffer(old.depth_stencil);
            }

            let framebuffer = match gl.create_framebuffer() {
                Ok(value) => value,
                Err(error) => {
                    log::error!("failed to create default framebuffer: {error}");
                    return;
                }
            };
            let color = match gl.create_texture() {
                Ok(value) => value,
                Err(error) => {
                    log::error!("failed to create default color texture: {error}");
                    return;
                }
            };
            let depth_stencil = match gl.create_renderbuffer() {
                Ok(value) => value,
                Err(error) => {
                    log::error!("failed to create default depth renderbuffer: {error}");
                    return;
                }
            };

            gl.bind_texture(glow::TEXTURE_2D, Some(color));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                self.width,
                self.height,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::NEAREST as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::NEAREST as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.bind_texture(glow::TEXTURE_2D, None);

            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth_stencil));
            gl.renderbuffer_storage(
                glow::RENDERBUFFER,
                glow::DEPTH_STENCIL,
                self.width,
                self.height,
            );
            gl.bind_renderbuffer(glow::RENDERBUFFER, None);

            gl.bind_framebuffer(FRAMEBUFFER, Some(framebuffer));
            gl.framebuffer_texture_2d(
                FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(color),
                0,
            );
            gl.framebuffer_renderbuffer(
                FRAMEBUFFER,
                glow::DEPTH_STENCIL_ATTACHMENT,
                glow::RENDERBUFFER,
                Some(depth_stencil),
            );
            let status = gl.check_framebuffer_status(FRAMEBUFFER);
            if status != FRAMEBUFFER_COMPLETE {
                log::error!("default framebuffer incomplete (status {status:#x})");
            }
            gl.bind_framebuffer(FRAMEBUFFER, Some(framebuffer));
            gl.viewport(0, 0, self.width, self.height);

            self.default_framebuffer = Some(DefaultFramebuffer {
                framebuffer,
                color,
                depth_stencil,
                width: self.width,
                height: self.height,
            });
        }
    }

    /// Draw to the window and swap while its framebuffer is still bound, then
    /// restore the game's GL state.
    pub fn present(&mut self, window_width: i32, window_height: i32, swap_buffers: impl FnOnce()) {
        let Some(default) = self.default_framebuffer.as_ref() else {
            swap_buffers();
            return;
        };
        let Some(blit) = self.blit.as_ref() else {
            swap_buffers();
            return;
        };
        let scale = (window_width.max(1) as f32 / default.width as f32)
            .min(window_height.max(1) as f32 / default.height as f32);
        let dest_width = ((default.width as f32 * scale).round() as i32).max(1);
        let dest_height = ((default.height as f32 * scale).round() as i32).max(1);
        let dest_x = (window_width - dest_width) / 2;
        let dest_y = (window_height - dest_height) / 2;
        let position = blit.position_location;
        let saved_attrib = self.attribs.get(&position).copied().unwrap_or_default();
        let saved_buffer = self.array_buffer;
        unsafe {
            let gl = &self.gl;
            let mut previous_clear = [0.0f32; 4];
            gl.get_parameter_f32_slice(glow::COLOR_CLEAR_VALUE, &mut previous_clear);
            let previous_viewport = {
                let mut viewport = [0i32; 4];
                gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
                viewport
            };
            let previous_program = gl.get_parameter_program(glow::CURRENT_PROGRAM);
            let previous_active_texture = gl.get_parameter_i32(glow::ACTIVE_TEXTURE);
            let previous_texture = gl.get_parameter_i32(glow::TEXTURE_BINDING_2D);
            let previous_blend = gl.is_enabled(glow::BLEND);
            let previous_depth = gl.is_enabled(glow::DEPTH_TEST);
            let previous_stencil = gl.is_enabled(glow::STENCIL_TEST);
            let previous_scissor = gl.is_enabled(glow::SCISSOR_TEST);

            gl.bind_framebuffer(FRAMEBUFFER, None);
            gl.viewport(0, 0, window_width.max(1), window_height.max(1));
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.clear_color(
                previous_clear[0],
                previous_clear[1],
                previous_clear[2],
                previous_clear[3],
            );
            gl.viewport(dest_x, dest_y, dest_width, dest_height);
            gl.disable(glow::BLEND);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::STENCIL_TEST);
            gl.disable(glow::SCISSOR_TEST);
            gl.use_program(Some(blit.program));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(blit.vbo));
            gl.enable_vertex_attrib_array(position);
            gl.vertex_attrib_pointer_f32(position, 2, glow::FLOAT, false, 0, 0);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(default.color));
            gl.uniform_1_i32(blit.texture_location.as_ref(), 0);
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);

            // macOS's flushBuffer needs the window framebuffer bound; restoring
            // the offscreen framebuffer first leaves the window blank.
            swap_buffers();

            let restore_buffer = saved_attrib
                .buffer
                .and_then(|id| self.buffers.get(&id))
                .copied();
            gl.bind_buffer(glow::ARRAY_BUFFER, restore_buffer);
            gl.vertex_attrib_pointer_f32(
                position,
                saved_attrib.size,
                saved_attrib.ty,
                saved_attrib.normalized,
                saved_attrib.stride,
                saved_attrib.offset,
            );
            if saved_attrib.enabled {
                gl.enable_vertex_attrib_array(position);
            } else {
                gl.disable_vertex_attrib_array(position);
            }
            let restore_array_buffer = saved_buffer.and_then(|id| self.buffers.get(&id)).copied();
            gl.bind_buffer(glow::ARRAY_BUFFER, restore_array_buffer);
            gl.active_texture(previous_active_texture as u32);
            gl.bind_texture(
                glow::TEXTURE_2D,
                std::num::NonZeroU32::new(previous_texture as u32).map(NativeTexture),
            );
            gl.use_program(previous_program);
            if previous_blend {
                gl.enable(glow::BLEND);
            }
            if previous_depth {
                gl.enable(glow::DEPTH_TEST);
            }
            if previous_stencil {
                gl.enable(glow::STENCIL_TEST);
            }
            if previous_scissor {
                gl.enable(glow::SCISSOR_TEST);
            }
            gl.viewport(
                previous_viewport[0],
                previous_viewport[1],
                previous_viewport[2],
                previous_viewport[3],
            );

            gl.bind_framebuffer(FRAMEBUFFER, Some(default.framebuffer));
        }
        self.array_buffer = saved_buffer;
        self.attribs.insert(position, saved_attrib);
    }

    pub fn read_pixels(&mut self, x: i32, y: i32, width: i32, height: i32, out: &mut [u8]) {
        let needed = (width * height * 4) as usize;
        if out.len() < needed {
            return;
        }
        unsafe {
            self.gl.read_pixels(
                x,
                y,
                width,
                height,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut out[..needed])),
            );
        }
    }

    pub fn read_default_pixels(&mut self) -> (i32, i32, Vec<u8>) {
        let mut data = vec![0u8; (self.width * self.height * 4) as usize];
        unsafe {
            let gl = &self.gl;
            if let Some(default) = self.default_framebuffer.as_ref() {
                gl.bind_framebuffer(FRAMEBUFFER, Some(default.framebuffer));
            }
            gl.read_pixels(
                0,
                0,
                self.width,
                self.height,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut data)),
            );
        }
        (self.width, self.height, data)
    }

    pub fn read_default_pixels_top_down(&mut self) -> (i32, i32, Vec<u8>) {
        let (width, height, mut data) = self.read_default_pixels();
        pixels::flip_rows(&mut data, width as usize, height as usize);
        (width, height, data)
    }

    fn insert<T>(&mut self, value: T, map: impl FnOnce(&mut Self) -> &mut HashMap<u32, T>) -> u32 {
        let handle = self.next_handle;
        self.next_handle += 1;
        map(self).insert(handle, value);
        handle
    }

    pub fn create_texture(&mut self) -> Option<u32> {
        let texture = unsafe { self.gl.create_texture() }.ok()?;
        Some(self.insert(texture, |state| &mut state.textures))
    }

    pub fn delete_texture(&mut self, handle: u32) {
        if let Some(texture) = self.textures.remove(&handle) {
            unsafe { self.gl.delete_texture(texture) };
        }
    }

    pub fn bind_texture(&mut self, target: u32, handle: Option<u32>) {
        let texture = handle.and_then(|id| self.textures.get(&id)).copied();
        unsafe { self.gl.bind_texture(target, texture) };
    }

    pub fn active_texture(&mut self, unit: u32) {
        unsafe { self.gl.active_texture(unit) };
    }

    pub fn tex_parameter_i32(&mut self, target: u32, parameter: u32, value: i32) {
        unsafe { self.gl.tex_parameter_i32(target, parameter, value) };
    }

    #[allow(clippy::too_many_arguments)]
    pub fn tex_image_2d(
        &mut self,
        target: u32,
        level: i32,
        internal_format: i32,
        width: i32,
        height: i32,
        border: i32,
        format: u32,
        ty: u32,
        source: Option<&[u8]>,
        options: UploadOptions,
    ) {
        let converted =
            source.map(|data| pixels::convert(data, width as usize, height as usize, options));
        let upload = match &converted {
            Some(Cow::Borrowed(data)) => glow::PixelUnpackData::Slice(Some(data)),
            Some(Cow::Owned(data)) => glow::PixelUnpackData::Slice(Some(data)),
            None => glow::PixelUnpackData::Slice(None),
        };
        unsafe {
            self.gl.tex_image_2d(
                target,
                level,
                internal_format,
                width,
                height,
                border,
                format,
                ty,
                upload,
            )
        };
    }

    #[allow(clippy::too_many_arguments)]
    pub fn tex_sub_image_2d(
        &mut self,
        target: u32,
        level: i32,
        x_offset: i32,
        y_offset: i32,
        width: i32,
        height: i32,
        format: u32,
        ty: u32,
        source: &[u8],
        options: UploadOptions,
    ) {
        let converted = pixels::convert(source, width as usize, height as usize, options);
        let upload = match &converted {
            Cow::Borrowed(data) => glow::PixelUnpackData::Slice(Some(data)),
            Cow::Owned(data) => glow::PixelUnpackData::Slice(Some(data)),
        };
        unsafe {
            self.gl.tex_sub_image_2d(
                target, level, x_offset, y_offset, width, height, format, ty, upload,
            )
        };
    }

    pub fn pixel_store_i32(&mut self, parameter: u32, value: i32) {
        const UNPACK_FLIP_Y_WEBGL: u32 = 0x9240;
        const UNPACK_PREMULTIPLY_ALPHA_WEBGL: u32 = 0x9241;
        match parameter {
            UNPACK_FLIP_Y_WEBGL => self.flip_y = value != 0,
            UNPACK_PREMULTIPLY_ALPHA_WEBGL => self.premultiply = value != 0,
            _ => {}
        }
        unsafe { self.gl.pixel_store_i32(parameter, value) };
    }

    pub fn upload_options(&self) -> UploadOptions {
        UploadOptions {
            flip_y: self.flip_y,
            premultiply: self.premultiply,
        }
    }

    pub fn viewport(&mut self, x: i32, y: i32, width: i32, height: i32) {
        unsafe { self.gl.viewport(x, y, width, height) };
    }

    pub fn clear_color(&mut self, red: f32, green: f32, blue: f32, alpha: f32) {
        unsafe { self.gl.clear_color(red, green, blue, alpha) };
    }

    pub fn clear(&mut self, mask: u32) {
        unsafe { self.gl.clear(mask) };
    }

    pub fn enable(&mut self, capability: u32) {
        unsafe { self.gl.enable(capability) };
    }

    pub fn disable(&mut self, capability: u32) {
        unsafe { self.gl.disable(capability) };
    }

    pub fn blend_func_separate(&mut self, srgb: u32, drgb: u32, salpha: u32, dalpha: u32) {
        unsafe { self.gl.blend_func_separate(srgb, drgb, salpha, dalpha) };
    }

    pub fn stencil_func(&mut self, func: u32, reference: i32, mask: u32) {
        unsafe { self.gl.stencil_func(func, reference, mask) };
    }

    pub fn stencil_op(&mut self, fail: u32, zfail: u32, zpass: u32) {
        unsafe { self.gl.stencil_op(fail, zfail, zpass) };
    }

    pub fn get_parameter_i32(&mut self, parameter: u32) -> i32 {
        unsafe { self.gl.get_parameter_i32(parameter) }
    }

    pub fn create_buffer(&mut self) -> Option<u32> {
        let buffer = unsafe { self.gl.create_buffer() }.ok()?;
        Some(self.insert(buffer, |state| &mut state.buffers))
    }

    pub fn delete_buffer(&mut self, handle: u32) {
        if let Some(buffer) = self.buffers.remove(&handle) {
            unsafe { self.gl.delete_buffer(buffer) };
        }
    }

    pub fn bind_buffer(&mut self, target: u32, handle: Option<u32>) {
        let buffer = handle.and_then(|id| self.buffers.get(&id)).copied();
        if target == glow::ARRAY_BUFFER {
            self.array_buffer = handle;
        }
        unsafe { self.gl.bind_buffer(target, buffer) };
    }

    pub fn buffer_data_size(&mut self, target: u32, size: i32, usage: u32) {
        unsafe { self.gl.buffer_data_size(target, size, usage) };
    }

    pub fn buffer_data_bytes(&mut self, target: u32, bytes: &[u8], usage: u32) {
        unsafe { self.gl.buffer_data_u8_slice(target, bytes, usage) };
    }

    pub fn buffer_sub_data(&mut self, target: u32, offset: i32, bytes: &[u8]) {
        unsafe { self.gl.buffer_sub_data_u8_slice(target, offset, bytes) };
    }

    pub fn enable_vertex_attrib_array(&mut self, index: u32) {
        self.attribs.entry(index).or_default().enabled = true;
        unsafe { self.gl.enable_vertex_attrib_array(index) };
    }

    pub fn disable_vertex_attrib_array(&mut self, index: u32) {
        self.attribs.entry(index).or_default().enabled = false;
        unsafe { self.gl.disable_vertex_attrib_array(index) };
    }

    pub fn vertex_attrib_pointer(
        &mut self,
        index: u32,
        size: i32,
        ty: u32,
        normalized: bool,
        stride: i32,
        offset: i32,
    ) {
        self.attribs.entry(index).or_default().size = size;
        self.attribs.entry(index).or_default().ty = ty;
        self.attribs.entry(index).or_default().normalized = normalized;
        self.attribs.entry(index).or_default().stride = stride;
        self.attribs.entry(index).or_default().offset = offset;
        self.attribs.entry(index).or_default().buffer = self.array_buffer;
        unsafe {
            self.gl
                .vertex_attrib_pointer_f32(index, size, ty, normalized, stride, offset)
        };
    }

    pub fn draw_arrays(&mut self, mode: u32, first: i32, count: i32) {
        unsafe { self.gl.draw_arrays(mode, first, count) };
    }

    pub fn draw_elements(&mut self, mode: u32, count: i32, ty: u32, offset: i32) {
        unsafe { self.gl.draw_elements(mode, count, ty, offset) };
    }

    pub fn create_shader(&mut self, kind: u32) -> Option<u32> {
        let shader = unsafe { self.gl.create_shader(kind) }.ok()?;
        let handle = self.insert(shader, |state| &mut state.shaders);
        self.shader_types.insert(handle, kind);
        Some(handle)
    }

    pub fn delete_shader(&mut self, handle: u32) {
        self.shader_types.remove(&handle);
        self.shader_logs.remove(&handle);
        if let Some(shader) = self.shaders.remove(&handle) {
            unsafe { self.gl.delete_shader(shader) };
        }
    }

    pub fn shader_source(&mut self, handle: u32, source: &str) -> Result<(), String> {
        let shader = self
            .shaders
            .get(&handle)
            .copied()
            .ok_or_else(|| format!("unknown shader {handle}"))?;
        let stage = match self.shader_types.get(&handle) {
            Some(&kind) if kind == glow::FRAGMENT_SHADER => shader::Stage::Fragment,
            _ => shader::Stage::Vertex,
        };
        match shader::translate(source, stage, self.profile) {
            Ok(translated) => {
                self.shader_logs.remove(&handle);
                unsafe { self.gl.shader_source(shader, &translated) };
                Ok(())
            }
            Err(error) => {
                self.shader_logs.insert(handle, error.clone());
                Err(error)
            }
        }
    }

    pub fn compile_shader(&mut self, handle: u32) {
        if let Some(shader) = self.shaders.get(&handle).copied() {
            unsafe { self.gl.compile_shader(shader) };
        }
    }

    pub fn get_shader_info_log(&mut self, handle: u32) -> String {
        if let Some(log) = self.shader_logs.get(&handle) {
            return log.clone();
        }
        self.shaders
            .get(&handle)
            .copied()
            .map(|shader| unsafe { self.gl.get_shader_info_log(shader) })
            .unwrap_or_default()
    }

    pub fn create_program(&mut self) -> Option<u32> {
        let program = unsafe { self.gl.create_program() }.ok()?;
        Some(self.insert(program, |state| &mut state.programs))
    }

    pub fn delete_program(&mut self, handle: u32) {
        self.invalidate_program(handle);
        if let Some(program) = self.programs.remove(&handle) {
            unsafe { self.gl.delete_program(program) };
        }
    }

    pub fn attach_shader(&mut self, program: u32, shader: u32) {
        let (Some(program), Some(shader)) = (
            self.programs.get(&program).copied(),
            self.shaders.get(&shader).copied(),
        ) else {
            return;
        };
        unsafe { self.gl.attach_shader(program, shader) };
    }

    pub fn bind_attrib_location(&mut self, program: u32, index: u32, name: &str) {
        if let Some(program) = self.programs.get(&program).copied() {
            unsafe { self.gl.bind_attrib_location(program, index, name) };
        }
    }

    pub fn link_program(&mut self, handle: u32) {
        self.invalidate_program(handle);
        if let Some(program) = self.programs.get(&handle).copied() {
            unsafe { self.gl.link_program(program) };
        }
    }

    pub fn get_link_status(&mut self, handle: u32) -> bool {
        self.programs
            .get(&handle)
            .copied()
            .map(|program| unsafe {
                self.gl
                    .get_program_parameter_i32(program, glow::LINK_STATUS)
                    != 0
            })
            .unwrap_or(false)
    }

    pub fn get_program_info_log(&mut self, handle: u32) -> String {
        self.programs
            .get(&handle)
            .copied()
            .map(|program| unsafe { self.gl.get_program_info_log(program) })
            .unwrap_or_default()
    }

    pub fn use_program(&mut self, handle: Option<u32>) {
        let program = handle.and_then(|id| self.programs.get(&id)).copied();
        unsafe { self.gl.use_program(program) };
    }

    pub fn get_attrib_location(&mut self, program: u32, name: &str) -> i32 {
        if let Some(location) = self
            .attrib_cache
            .get(&program)
            .and_then(|cache| cache.get(name))
        {
            return *location;
        }
        let Some(native) = self.programs.get(&program).copied() else {
            return -1;
        };
        let location = unsafe { self.gl.get_attrib_location(native, name) }
            .map_or(-1, |location| location as i32);
        self.attrib_cache
            .entry(program)
            .or_default()
            .insert(name.to_string(), location);
        location
    }

    pub fn get_uniform_location(&mut self, program: u32, name: &str) -> i32 {
        if let Some(handle) = self
            .uniform_cache
            .get(&program)
            .and_then(|cache| cache.get(name))
        {
            return *handle;
        }
        let Some(native) = self.programs.get(&program).copied() else {
            return -1;
        };
        let Some(location) = (unsafe { self.gl.get_uniform_location(native, name) }) else {
            return -1;
        };
        let handle = self.next_handle;
        self.next_handle += 1;
        self.uniform_locations.insert(handle, location);
        self.uniform_cache
            .entry(program)
            .or_default()
            .insert(name.to_string(), handle as i32);
        handle as i32
    }

    pub fn resource_counts(&self) -> (usize, usize, usize) {
        (
            self.uniform_locations.len(),
            self.uniform_cache.values().map(HashMap::len).sum(),
            self.attrib_cache.values().map(HashMap::len).sum(),
        )
    }

    fn invalidate_program(&mut self, program: u32) {
        if let Some(cache) = self.uniform_cache.remove(&program) {
            for handle in cache.values() {
                self.uniform_locations.remove(&(*handle as u32));
            }
        }
        self.attrib_cache.remove(&program);
    }

    fn location(&self, handle: i32) -> Option<&glow::UniformLocation> {
        if handle < 0 {
            None
        } else {
            self.uniform_locations.get(&(handle as u32))
        }
    }

    pub fn uniform_1f(&mut self, location: i32, value: f32) {
        unsafe { self.gl.uniform_1_f32(self.location(location), value) };
    }

    pub fn uniform_1i(&mut self, location: i32, value: i32) {
        unsafe { self.gl.uniform_1_i32(self.location(location), value) };
    }

    pub fn uniform_2f(&mut self, location: i32, x: f32, y: f32) {
        unsafe { self.gl.uniform_2_f32(self.location(location), x, y) };
    }

    pub fn uniform_3f(&mut self, location: i32, x: f32, y: f32, z: f32) {
        unsafe { self.gl.uniform_3_f32(self.location(location), x, y, z) };
    }

    pub fn uniform_1fv(&mut self, location: i32, values: &[f32]) {
        unsafe { self.gl.uniform_1_f32_slice(self.location(location), values) };
    }

    pub fn uniform_2fv(&mut self, location: i32, values: &[f32]) {
        unsafe { self.gl.uniform_2_f32_slice(self.location(location), values) };
    }

    pub fn uniform_3fv(&mut self, location: i32, values: &[f32]) {
        unsafe { self.gl.uniform_3_f32_slice(self.location(location), values) };
    }

    pub fn uniform_matrix_4fv(&mut self, location: i32, values: &[f32]) {
        unsafe {
            self.gl
                .uniform_matrix_4_f32_slice(self.location(location), false, values)
        };
    }

    pub fn create_framebuffer(&mut self) -> Option<u32> {
        let framebuffer = unsafe { self.gl.create_framebuffer() }.ok()?;
        Some(self.insert(framebuffer, |state| &mut state.framebuffers))
    }

    pub fn delete_framebuffer(&mut self, handle: u32) {
        if let Some(framebuffer) = self.framebuffers.remove(&handle) {
            unsafe { self.gl.delete_framebuffer(framebuffer) };
        }
    }

    pub fn bind_framebuffer(&mut self, target: u32, handle: Option<u32>) {
        let framebuffer = match handle {
            Some(id) => self.framebuffers.get(&id).copied(),
            None => self.default_framebuffer.as_ref().map(|fbo| fbo.framebuffer),
        };
        unsafe { self.gl.bind_framebuffer(target, framebuffer) };
    }

    pub fn create_renderbuffer(&mut self) -> Option<u32> {
        let renderbuffer = unsafe { self.gl.create_renderbuffer() }.ok()?;
        Some(self.insert(renderbuffer, |state| &mut state.renderbuffers))
    }

    pub fn delete_renderbuffer(&mut self, handle: u32) {
        if let Some(renderbuffer) = self.renderbuffers.remove(&handle) {
            unsafe { self.gl.delete_renderbuffer(renderbuffer) };
        }
    }

    pub fn bind_renderbuffer(&mut self, target: u32, handle: Option<u32>) {
        let renderbuffer = handle.and_then(|id| self.renderbuffers.get(&id)).copied();
        unsafe { self.gl.bind_renderbuffer(target, renderbuffer) };
    }

    pub fn renderbuffer_storage(
        &mut self,
        target: u32,
        internal_format: u32,
        width: i32,
        height: i32,
    ) {
        unsafe {
            self.gl
                .renderbuffer_storage(target, internal_format, width, height)
        };
    }

    pub fn framebuffer_texture_2d(
        &mut self,
        target: u32,
        attachment: u32,
        texture_target: u32,
        texture: Option<u32>,
        level: i32,
    ) {
        let texture = texture.and_then(|id| self.textures.get(&id)).copied();
        unsafe {
            self.gl
                .framebuffer_texture_2d(target, attachment, texture_target, texture, level)
        };
    }

    pub fn framebuffer_renderbuffer(
        &mut self,
        target: u32,
        attachment: u32,
        renderbuffer_target: u32,
        renderbuffer: Option<u32>,
    ) {
        let renderbuffer = renderbuffer
            .and_then(|id| self.renderbuffers.get(&id))
            .copied();
        unsafe {
            self.gl
                .framebuffer_renderbuffer(target, attachment, renderbuffer_target, renderbuffer)
        };
    }

    pub fn check_framebuffer_status(&mut self, target: u32) -> u32 {
        unsafe { self.gl.check_framebuffer_status(target) }
    }
}

#[cfg(test)]
mod tests {
    use super::shader;

    #[test]
    fn translates_single_line_shaders() {
        let source = "precision mediump float;varying vec2 v;void main(){gl_FragColor=vec4(1.0);}";
        let translated = shader::translate(
            source,
            shader::Stage::Vertex,
            shader::Profile::Compatibility,
        )
        .unwrap();
        assert!(!translated.contains("precision"));
        assert!(translated.contains("gl_FragColor"));
    }

    #[test]
    fn removes_version_100() {
        let source = "#version 100\nattribute vec2 a;void main(){gl_Position=vec4(a,0.0,1.0);}";
        let translated = shader::translate(
            source,
            shader::Stage::Vertex,
            shader::Profile::Compatibility,
        )
        .unwrap();
        assert!(translated.starts_with("#version 120\n"));
        assert_eq!(translated.matches("#version").count(), 1);
        assert!(translated.contains("attribute vec2 a"));
    }

    #[test]
    fn rejects_es3_shaders() {
        assert!(shader::translate(
            "#version 300 es\nvoid main(){}",
            shader::Stage::Vertex,
            shader::Profile::Core
        )
        .is_err());
    }
}
