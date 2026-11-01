/// Адаптер, устройство и очередь: общие у `Canvas`, `FrameRenderer` и вывода.
#[derive(Clone)]
pub struct Gpu {
    /// Адаптер, на котором открыто устройство.
    pub adapter: wgpu::Adapter,
    /// Устройство.
    pub device: wgpu::Device,
    /// Очередь устройства.
    pub queue: wgpu::Queue,
}

impl Gpu {
    /// Открывает устройство на адаптере.
    ///
    /// # Panics
    /// Если устройство не открылось.
    #[must_use]
    pub fn new(adapter: wgpu::Adapter) -> Self {
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("request device");
        Self {
            adapter,
            device,
            queue,
        }
    }

    /// Адаптер без поверхности, для офскрин-рендера.
    ///
    /// # Errors
    /// Адаптера нет.
    pub fn headless() -> Result<Self, wgpu::RequestAdapterError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?;
        Ok(Self::new(adapter))
    }
}
