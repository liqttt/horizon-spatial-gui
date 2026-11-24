//! Spatial GUI — renders inventory and container screens as a 3D plane in the world
//! with smooth third-person over-the-shoulder camera transitions.
//!
//! Horizon Rust port of Spatial GUI (MIT License), originally created by tastytrash
//! (https://github.com/tastytrash/Spatial-GUI).

#![deny(unsafe_code)]

pub mod math;
pub mod transform;

use math::{SpatialConfig, ViewMode, runtime};
use mod_api::{
    self, CameraPoseEvent, ClientHost, ClientTickPostEvent, ClipBlock, ClipFluid, Flow, Hooks,
    KeyConflictContext, KeyRegistration, MenuBackgroundEvent, Registrar, RegistrationContext,
    ScreenClosingEvent, ScreenOpening, ScreenOpeningEvent, ScreenTransparentBackgroundEvent,
    SettingChangedEvent, SettingSpec,
};
use std::sync::atomic::{AtomicBool, Ordering};

const MOD_ID: &str = "spatial_gui";
static SETTINGS_SYNCED: AtomicBool = AtomicBool::new(false);

fn clip_camera_position(
    host: &dyn ClientHost,
    level: Option<mod_api::LevelRef>,
    from: [f64; 3],
    to: [f64; 3],
) -> [f64; 3] {
    let Some(level) = level else {
        return from;
    };
    let dx = to[0] - from[0];
    let dy = to[1] - from[1];
    let dz = to[2] - from[2];
    let dist = (dx * dx + dy * dy + dz * dz).sqrt();
    if dist <= 1e-4 {
        return to;
    }
    let Some(hit) = level.clip(host, from, to, ClipBlock::Collider, ClipFluid::None) else {
        return from;
    };
    if hit.miss {
        return to;
    }
    let hx = hit.location[0] - from[0];
    let hy = hit.location[1] - from[1];
    let hz = hit.location[2] - from[2];
    let hit_dist = (hx * hx + hy * hy + hz * hz).sqrt();
    let safe_dist = (hit_dist - 0.12).max(0.0);
    let scale = safe_dist / dist;
    [
        from[0] + dx * scale,
        from[1] + dy * scale,
        from[2] + dz * scale,
    ]
}

#[derive(Default)]
struct SpatialGuiMod;

fn sync_all_settings(host: &dyn ClientHost, cfg: &mut SpatialConfig) {
    if let Some(v) = host.setting_bool("enabled") {
        cfg.enabled = v;
    }
    if let Some(v) = host.setting_choice("view_mode") {
        cfg.view_mode = ViewMode::from_choice(v);
    }
    if let Some(v) = host.setting_bool("mirror_third_person") {
        cfg.mirror_third_person = v;
    }
    if let Some(v) = host.setting_bool("remove_backdrop") {
        cfg.remove_backdrop = v;
    }
    if let Some(v) = host.setting_bool("drop_shadow") {
        cfg.drop_shadow = v;
    }
    if let Some(v) = host.setting_bool("subdivide") {
        cfg.subdivide = v;
    }
    if let Some(v) = host.setting_f64("camera_distance") {
        cfg.camera_distance = v as f32;
    }
    if let Some(v) = host.setting_f64("camera_side_offset") {
        cfg.camera_side_offset = v as f32;
    }
    if let Some(v) = host.setting_f64("camera_height_offset") {
        cfg.camera_height_offset = v as f32;
    }
    if let Some(v) = host.setting_f64("screen_distance") {
        cfg.screen_distance = v as f32;
    }
    if let Some(v) = host.setting_f64("screen_side_offset") {
        cfg.screen_side_offset = v as f32;
    }
    if let Some(v) = host.setting_f64("screen_height_offset") {
        cfg.screen_height_offset = v as f32;
    }
    if let Some(v) = host.setting_f64("screen_yaw_offset") {
        cfg.screen_yaw_offset = v as f32;
    }
    if let Some(v) = host.setting_f64("screen_scale") {
        cfg.screen_scale = v as f32;
    }
    if let Some(v) = host.setting_f64("transition_ms") {
        cfg.transition_ms = v as f32;
    }
    if let Some(v) = host.setting_f64("fp_screen_distance") {
        cfg.fp_screen_distance = v as f32;
    }
    if let Some(v) = host.setting_f64("fp_screen_scale") {
        cfg.fp_screen_scale = v as f32;
    }
    if let Some(v) = host.setting_f64("fp_screen_yaw_offset") {
        cfg.fp_screen_yaw_offset = v as f32;
    }
}

#[mod_api::horizon_mod]
impl Hooks for SpatialGuiMod {
    fn register(&self, registrar: &mut dyn Registrar, _cx: &RegistrationContext) {
        let _ = registrar.register_setting(
            SettingSpec::toggle("enabled", "spatial_gui.setting.enabled", false)
                .tooltip("spatial_gui.setting.enabled.tooltip"),
        );
        let _ = registrar.register_setting(
            SettingSpec::choice(
                "view_mode",
                "spatial_gui.setting.view_mode",
                [
                    "spatial_gui.view_mode.third_person",
                    "spatial_gui.view_mode.first_person",
                ],
                0,
            )
            .tooltip("spatial_gui.setting.view_mode.tooltip"),
        );
        let _ = registrar.register_setting(
            SettingSpec::toggle(
                "mirror_third_person",
                "spatial_gui.setting.mirror_third_person",
                false,
            )
            .tooltip("spatial_gui.setting.mirror_third_person.tooltip"),
        );
        let _ = registrar.register_setting(
            SettingSpec::toggle("remove_backdrop", "spatial_gui.setting.remove_backdrop", true)
                .tooltip("spatial_gui.setting.remove_backdrop.tooltip"),
        );
        let _ = registrar.register_setting(
            SettingSpec::toggle("drop_shadow", "spatial_gui.setting.drop_shadow", true)
                .tooltip("spatial_gui.setting.drop_shadow.tooltip"),
        );
        let _ = registrar.register_setting(
            SettingSpec::range(
                "camera_distance",
                "spatial_gui.setting.camera_distance",
                0.5,
                6.0,
                0.05,
                1.7,
            )
            .tooltip("spatial_gui.setting.camera_distance.tooltip"),
        );
        let _ = registrar.register_setting(
            SettingSpec::range(
                "camera_side_offset",
                "spatial_gui.setting.camera_side_offset",
                -4.0,
                4.0,
                0.05,
                -1.2,
            )
            .tooltip("spatial_gui.setting.camera_side_offset.tooltip"),
        );
        let _ = registrar.register_setting(
            SettingSpec::range(
                "camera_height_offset",
                "spatial_gui.setting.camera_height_offset",
                -1.0,
                4.0,
                0.05,
                1.5,
            )
            .tooltip("spatial_gui.setting.camera_height_offset.tooltip"),
        );
        let _ = registrar.register_setting(
            SettingSpec::range(
                "screen_distance",
                "spatial_gui.setting.screen_distance",
                0.5,
                8.0,
                0.05,
                2.5,
            )
            .tooltip("spatial_gui.setting.screen_distance.tooltip"),
        );
        let _ = registrar.register_setting(
            SettingSpec::range(
                "screen_side_offset",
                "spatial_gui.setting.screen_side_offset",
                -4.0,
                4.0,
                0.05,
                -0.6,
            )
            .tooltip("spatial_gui.setting.screen_side_offset.tooltip"),
        );
        let _ = registrar.register_setting(
            SettingSpec::range(
                "screen_height_offset",
                "spatial_gui.setting.screen_height_offset",
                -3.0,
                3.0,
                0.05,
                -0.4,
            )
            .tooltip("spatial_gui.setting.screen_height_offset.tooltip"),
        );
        let _ = registrar.register_setting(
            SettingSpec::range(
                "screen_yaw_offset",
                "spatial_gui.setting.screen_yaw_offset",
                0.0,
                360.0,
                1.0,
                160.0,
            )
            .tooltip("spatial_gui.setting.screen_yaw_offset.tooltip"),
        );
        let _ = registrar.register_setting(
            SettingSpec::range(
                "screen_scale",
                "spatial_gui.setting.screen_scale",
                0.5,
                8.0,
                0.05,
                3.5,
            )
            .tooltip("spatial_gui.setting.screen_scale.tooltip"),
        );
        let _ = registrar.register_setting(
            SettingSpec::range(
                "transition_ms",
                "spatial_gui.setting.transition_ms",
                50.0,
                1500.0,
                10.0,
                300.0,
            )
            .tooltip("spatial_gui.setting.transition_ms.tooltip"),
        );
        let _ = registrar.register_setting(
            SettingSpec::toggle("subdivide", "spatial_gui.setting.subdivide", true)
                .tooltip("spatial_gui.setting.subdivide.tooltip"),
        );
        let _ = registrar.register_setting(
            SettingSpec::range(
                "fp_screen_distance",
                "spatial_gui.setting.fp_screen_distance",
                0.5,
                5.0,
                0.05,
                1.35,
            )
            .tooltip("spatial_gui.setting.fp_screen_distance.tooltip"),
        );
        let _ = registrar.register_setting(
            SettingSpec::range(
                "fp_screen_scale",
                "spatial_gui.setting.fp_screen_scale",
                0.5,
                6.0,
                0.05,
                2.35,
            )
            .tooltip("spatial_gui.setting.fp_screen_scale.tooltip"),
        );
        let _ = registrar.register_setting(
            SettingSpec::range(
                "fp_screen_yaw_offset",
                "spatial_gui.setting.fp_screen_yaw_offset",
                120.0,
                240.0,
                1.0,
                172.0,
            )
            .tooltip("spatial_gui.setting.fp_screen_yaw_offset.tooltip"),
        );

        registrar.register_key_mapping(
            KeyRegistration::new("toggle", "key.keyboard.f8")
                .label("key.spatial_gui.toggle")
                .category("key.categories.spatial_gui")
                .context(KeyConflictContext::Universal),
        );
    }

    fn setting_changed(&self, host: &mut dyn ClientHost, _event: &mut SettingChangedEvent<'_>) {
        if let Ok(mut rt) = runtime().lock() {
            sync_all_settings(host, &mut rt.config);
            SETTINGS_SYNCED.store(true, Ordering::Relaxed);
        }
    }

    fn client_tick_post(&self, host: &mut dyn ClientHost, _event: &mut ClientTickPostEvent) {
        if !SETTINGS_SYNCED.swap(true, Ordering::Relaxed)
            && let Ok(mut rt) = runtime().lock()
        {
            sync_all_settings(host, &mut rt.config);
        }
        if host.consume_click(MOD_ID, "toggle")
            && let Ok(mut rt) = runtime().lock()
        {
            rt.config.enabled = !rt.config.enabled;
            let val = if rt.config.enabled { "true" } else { "false" };
            host.set_setting(MOD_ID, "enabled", val);
        }
        if let Some(player) = host.local_player() {
            let ent = player.entity();
            if let (Some(eye), Some(rot)) = (ent.eye_position(host), ent.rotation(host))
                && let Ok(mut rt) = runtime().lock()
            {
                let h = ent.eye_height(host).unwrap_or(1.62) as f32;
                rt.record_tick_pose(eye, h, rot);
            }
        }
    }

    fn screen_opening(
        &self,
        _host: &dyn ClientHost,
        event: &mut ScreenOpeningEvent<'_>,
    ) -> Option<ScreenOpening> {
        let is_container = event.new.menu.is_some()
            || matches!(&*event.new.id, "inventory" | "creative");
        if let Ok(mut rt) = runtime().lock() {
            rt.set_container_open(is_container);
        }
        None
    }

    fn screen_closing(&self, _host: &dyn ClientHost, _event: &mut ScreenClosingEvent<'_>) {
        if let Ok(mut rt) = runtime().lock() {
            rt.set_container_open(false);
        }
    }

    fn camera_pose(&self, host: &dyn ClientHost, event: &mut CameraPoseEvent<'_>) {
        let fov = host.camera().map(|c| c.fov_degrees).unwrap_or(70.0);
        let cur_type = host.camera_type();
        let is_singleplayer = host
            .client_session()
            .client_server()
            .is_some_and(|s| s.integrated_local && s.address.is_none());
        let level = host.local_player().and_then(|p| p.level(host));
        let override_res = match runtime().lock() {
            Ok(mut rt) => rt.compute_camera_pose(
                event.partial,
                *event.position,
                event.yaw,
                event.pitch,
                cur_type,
                fov,
                is_singleplayer,
                |from, to| clip_camera_position(host, level, from, to),
            ),
            Err(_) => return,
        };
        if let Some((pos, yaw, pitch, desired_type)) = override_res {
            *event.position = pos;
            event.yaw = yaw;
            event.pitch = pitch;
            if desired_type != cur_type {
                host.set_camera_type(desired_type);
            }
        }
    }

    fn menu_background(&self, _host: &dyn ClientHost, event: &mut MenuBackgroundEvent<'_>) {
        if let Ok(rt) = runtime().lock()
            && rt.is_active()
            && rt.config.remove_backdrop
        {
            *event.radius = None;
        }
    }

    fn screen_transparent_background(
        &self,
        _host: &dyn ClientHost,
        event: &mut ScreenTransparentBackgroundEvent<'_>,
    ) -> Flow {
        if let Ok(rt) = runtime().lock()
            && rt.is_active()
            && rt.config.remove_backdrop
            && matches!(
                event.screen,
                "inventory" | "creative" | "station" | "beacon" | "merchant"
            )
        {
            return Flow::Cancel;
        }
        Flow::Continue
    }
}

/// Called from `perspective::Game::camera_view` (`RETURN` mixin).
pub fn on_camera_view(
    is_container: bool,
    eye_pos: [f64; 3],
    eye_height: f32,
    eye_rot: [f32; 2],
) {
    if let Ok(mut rt) = runtime().lock() {
        rt.record_camera_view(is_container, eye_pos, eye_height, eye_rot);
    }
}

/// Called from `Game::hud` (`HEAD` mixin) to build the world plane and re-project `game.cursor`.
pub fn on_hud_head(
    is_container_screen: bool,
    gui_w: f32,
    gui_h: f32,
    current_cursor: (f32, f32),
) -> Option<(f32, f32)> {
    let Ok(mut rt) = runtime().lock() else {
        return None;
    };
    rt.step_hud(is_container_screen, gui_w, gui_h, current_cursor)
}

/// Called from `Game::inventory_drag_move` (`HEAD` mixin) on container mouse movement.
pub fn on_cursor_moved(raw_x: f32, raw_y: f32) -> (f32, f32) {
    let Ok(mut rt) = runtime().lock() else {
        return (raw_x, raw_y);
    };
    rt.update_cursor(raw_x, raw_y)
}

/// Returns `(plane, gui_w, gui_h, subdivide, drop_shadow)` if Spatial GUI is active this frame.
pub fn active_plane() -> Option<(math::WorldPlane, f32, f32, bool, bool)> {
    let Ok(rt) = runtime().lock() else {
        return None;
    };
    if !rt.is_active() {
        return None;
    }
    rt.current_plane.map(|plane| {
        (
            plane,
            plane.gui_w,
            plane.gui_h,
            rt.config.subdivide,
            rt.config.drop_shadow,
        )
    })
}
