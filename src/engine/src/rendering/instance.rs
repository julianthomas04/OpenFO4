pub fn new() -> wgpu::Instance {
    let flags = wgpu::InstanceFlags::from_env_or_default();
    let backends = wgpu::Backends::from_env().unwrap_or(wgpu::Backends::PRIMARY);
    tracing::info!("wgpu instance flags: {flags:?}, backends: {backends:?}");
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends,
        flags,
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        backend_options: wgpu::BackendOptions::default(),
        display: None,
    })
}
