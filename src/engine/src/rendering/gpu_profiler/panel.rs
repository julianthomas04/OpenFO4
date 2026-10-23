use super::{GpuPass, GpuProfiler, ProfilerStatus};

pub fn gpu_profiler_section(ui: &mut egui::Ui, profiler: &GpuProfiler) {
    match profiler.status() {
        ProfilerStatus::Unsupported => {
            ui.label("GPU timing: unsupported by this adapter");
        }
        ProfilerStatus::Active => {
            ui.label(format!(
                "GPU timing: active ({} skipped frames, {} invalid samples)",
                profiler.skipped_frames(),
                profiler.invalid_samples(),
            ));
            pass_table(ui, profiler);
        }
    }
}

fn pass_table(ui: &mut egui::Ui, profiler: &GpuProfiler) {
    egui::Grid::new("gpu_pass_times").show(ui, |ui| {
        for pass in GpuPass::ALL {
            ui.label(pass.label());
            match profiler.smoothed_ms(*pass) {
                Some(ms) => ui.label(format!("{ms:.3} ms")),
                None => ui.label("-"),
            };
            ui.end_row();
        }
        ui.label("total");
        ui.label(format!("{:.3} ms", profiler.total_ms()));
        ui.end_row();
    });
}
