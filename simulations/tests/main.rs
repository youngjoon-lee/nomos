use simulations::{app::SimulationApp, settings::SimulationSettings, streaming::StreamType};

fn main() {
    let settings = SimulationSettings {
        wards: todo!(),
        network_settings: todo!(),
        overlay_settings: todo!(),
        node_settings: todo!(),
        runner_settings: todo!(),
        stream_settings: todo!(),
        node_count: todo!(),
        views_count: todo!(),
        leaders_count: todo!(),
        seed: todo!(),
        sim_duration: todo!(),
    };
    SimulationApp::run(settings, StreamType::Naive)?;
}
