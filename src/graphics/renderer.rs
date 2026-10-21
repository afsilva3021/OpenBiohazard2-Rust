pub struct Renderer {
    viewport_width: u32,
    viewport_height: u32,
}

impl Renderer {
    pub fn new(viewport_width: u32, viewport_height: u32) -> Self {
        let mut renderer = Self {
            viewport_width: 0,
            viewport_height: 0,
        };
        renderer.resize(viewport_width, viewport_height);
        renderer
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.viewport_width = width.max(1);
        self.viewport_height = height.max(1);

        unsafe {
            gl::Viewport(
                0,
                0,
                self.viewport_width as i32,
                self.viewport_height as i32,
            );
        }
    }

    pub fn render(&self) {
        unsafe {
            gl::ClearColor(0.02, 0.04, 0.08, 1.0);
            gl::Clear(gl::COLOR_BUFFER_BIT);
        }
    }
}
