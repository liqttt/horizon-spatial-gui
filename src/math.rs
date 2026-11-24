//! Authentic world-space 3D GUI plane projection, camera swing state machine, and ray-plane unprojection.
//!
//! Ported from `tastytrash/Spatial-GUI` (`CameraMixin.java`, `RenderUtil.java`, `AnimationUtil.java`,
//! `SpatialGUIConfig.java`) under the MIT License.

use mod_api::CameraType;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// Camera perspective mode while Spatial GUI is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    ThirdPerson = 0,
    FirstPerson = 1,
}

impl ViewMode {
    pub fn from_choice(idx: u32) -> Self {
        match idx {
            1 => Self::FirstPerson,
            _ => Self::ThirdPerson,
        }
    }
}

/// Configuration mirroring `tastytrash/Spatial-GUI` defaults.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpatialConfig {
    pub enabled: bool,
    pub view_mode: ViewMode,
    pub mirror_third_person: bool,
    pub remove_backdrop: bool,
    pub drop_shadow: bool,
    pub subdivide: bool,

    // Third-person camera
    pub camera_distance: f32,
    pub camera_side_offset: f32,
    pub camera_height_offset: f32,
    pub camera_orbit_yaw: f32,
    pub camera_target_pitch: f32,
    pub transition_ms: f32,
    pub parallax_yaw: f32,
    pub parallax_pitch: f32,

    // Third-person world screen
    pub screen_distance: f32,
    pub screen_side_offset: f32,
    pub screen_height_offset: f32,
    pub screen_yaw_offset: f32,
    pub screen_pitch_offset: f32,
    pub screen_scale: f32,

    // First-person world screen
    pub fp_screen_distance: f32,
    pub fp_screen_scale: f32,
    pub fp_screen_yaw_offset: f32,
}

impl Default for SpatialConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            view_mode: ViewMode::ThirdPerson,
            mirror_third_person: false,
            remove_backdrop: true,
            drop_shadow: true,
            subdivide: true,

            camera_distance: 1.7,
            camera_side_offset: -1.2,
            camera_height_offset: 1.5,
            camera_orbit_yaw: 0.0,
            camera_target_pitch: 10.0,
            transition_ms: 300.0,
            parallax_yaw: 0.10,
            parallax_pitch: 0.03,

            screen_distance: 2.5,
            screen_side_offset: -0.6,
            screen_height_offset: -0.4,
            screen_yaw_offset: 160.0,
            screen_pitch_offset: 0.0,
            screen_scale: 3.5,

            fp_screen_distance: 1.35,
            fp_screen_scale: 2.35,
            fp_screen_yaw_offset: 172.0,
        }
    }
}

#[inline]
fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
fn lerp_f32(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
fn lerp_f64(a: f64, b: f64, t: f32) -> f64 {
    a + (b - a) * (t as f64)
}

#[inline]
fn rot_lerp(start: f32, end: f32, t: f32) -> f32 {
    let diff = ((end - start) % 360.0 + 540.0) % 360.0 - 180.0;
    start + diff * t
}

#[inline]
fn ease_out_cubic(t: f32) -> f32 {
    let t1 = t.clamp(0.0, 1.0) - 1.0;
    t1 * t1 * t1 + 1.0
}

/// Builds a YXZ quaternion `(x, y, z, w)` matching `glam::Quat::from_euler(YXZ, y_rot, x_rot, 0.0)`.
#[inline]
fn quat_yxz(y_rad: f32, x_rad: f32) -> [f32; 4] {
    let (sy, cy) = (y_rad * 0.5).sin_cos();
    let (sx, cx) = (x_rad * 0.5).sin_cos();
    // Q = Q_y * Q_x
    [cy * sx, sy * cx, -sy * sx, cy * cx]
}

#[inline]
fn quat_rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let [qx, qy, qz, qw] = q;
    let tx = 2.0 * (qy * v[2] - qz * v[1]);
    let ty = 2.0 * (qz * v[0] - qx * v[2]);
    let tz = 2.0 * (qx * v[1] - qy * v[0]);
    [
        v[0] + qw * tx + (qy * tz - qz * ty),
        v[1] + qw * ty + (qz * tx - qx * tz),
        v[2] + qw * tz + (qx * ty - qy * tx),
    ]
}

/// Horizon camera view basis matching `horizon_client_render::camera::view_matrix(yaw, pitch)`.
///
/// Returns `(cam_q, cam_right, cam_up, cam_back)` in world coordinates.
pub fn view_basis(yaw_deg: f32, pitch_deg: f32) -> ([f32; 4], [f32; 3], [f32; 3], [f32; 3]) {
    let q = quat_yxz(
        std::f32::consts::PI - yaw_deg.to_radians(),
        -pitch_deg.to_radians(),
    );
    // `view_matrix` uses `q_conj`, so world-from-camera rotation is `q`.
    let right = quat_rotate(q, [1.0, 0.0, 0.0]);
    let up = quat_rotate(q, [0.0, 1.0, 0.0]);
    let back = quat_rotate(q, [0.0, 0.0, 1.0]);
    (q, right, up, back)
}

/// A 3D GUI plane anchored in world space and projected through the active Horizon camera.
#[derive(Clone, Copy, Debug)]
pub struct WorldPlane {
    pub center: [f64; 3],
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub normal: [f32; 3],
    pub half_w: f32,
    pub half_h: f32,
    pub cam_pos: [f64; 3],
    pub cam_q: [f32; 4],
    pub cam_right: [f32; 3],
    pub cam_up: [f32; 3],
    pub cam_back: [f32; 3],
    pub tan_half_fovy: f32,
    pub aspect: f32,
    pub gui_w: f32,
    pub gui_h: f32,
}

impl WorldPlane {
    /// Projects a 2D GUI coordinate `(gx, gy)` onto the 3D world plane and back to screen GUI pixels.
    pub fn project(&self, gx: f32, gy: f32) -> Option<(f32, f32)> {
        self.project_offset(gx, gy, [0.0, 0.0, 0.0])
    }

    /// Projects `(gx, gy)` with an extra world-space offset (used for 3D drop shadow).
    pub fn project_shadow(&self, gx: f32, gy: f32) -> Option<(f32, f32)> {
        // Offset slightly behind the plane and down-right in plane space.
        let offset = [
            -self.normal[0] * 0.028 + self.right[0] * 0.024 - self.up[0] * 0.028,
            -self.normal[1] * 0.028 + self.right[1] * 0.024 - self.up[1] * 0.028,
            -self.normal[2] * 0.028 + self.right[2] * 0.024 - self.up[2] * 0.028,
        ];
        self.project_offset(gx, gy, offset)
    }

    fn project_offset(&self, gx: f32, gy: f32, world_offset: [f32; 3]) -> Option<(f32, f32)> {
        let u = (gx / self.gui_w) * 2.0 - 1.0;
        let v = 1.0 - (gy / self.gui_h) * 2.0;
        let lx = u * self.half_w;
        let ly = v * self.half_h;

        let dx = (self.center[0] - self.cam_pos[0]) as f32
            + lx * self.right[0]
            + ly * self.up[0]
            + world_offset[0];
        let dy = (self.center[1] - self.cam_pos[1]) as f32
            + lx * self.right[1]
            + ly * self.up[1]
            + world_offset[1];
        let dz = (self.center[2] - self.cam_pos[2]) as f32
            + lx * self.right[2]
            + ly * self.up[2]
            + world_offset[2];
        let d = [dx, dy, dz];

        let cx = dot3(d, self.cam_right);
        let cy = dot3(d, self.cam_up);
        let cz = dot3(d, self.cam_back);
        let depth = -cz;
        if depth <= 0.02 {
            return None;
        }
        let ndc_y = cy / (depth * self.tan_half_fovy);
        let ndc_x = cx / (depth * self.tan_half_fovy * self.aspect);
        let sx = (ndc_x + 1.0) * 0.5 * self.gui_w;
        let sy = (1.0 - ndc_y) * 0.5 * self.gui_h;
        Some((sx, sy))
    }

    /// Intersects a screen-space cursor ray `(sx, sy)` with the 3D world plane to recover GUI `(gx, gy)`.
    pub fn unproject(&self, sx: f32, sy: f32) -> Option<(f32, f32)> {
        let ndc_x = (sx / self.gui_w) * 2.0 - 1.0;
        let ndc_y = 1.0 - (sy / self.gui_h) * 2.0;
        let d_cam = [
            ndc_x * self.tan_half_fovy * self.aspect,
            ndc_y * self.tan_half_fovy,
            -1.0,
        ];
        let d_world = quat_rotate(self.cam_q, d_cam);
        let denom = dot3(d_world, self.normal);
        if denom.abs() <= 1e-5 {
            return None;
        }
        let to_center = [
            (self.center[0] - self.cam_pos[0]) as f32,
            (self.center[1] - self.cam_pos[1]) as f32,
            (self.center[2] - self.cam_pos[2]) as f32,
        ];
        let t = dot3(to_center, self.normal) / denom;
        if t <= 0.02 {
            return None;
        }
        let delta = [
            d_world[0] * t - to_center[0],
            d_world[1] * t - to_center[1],
            d_world[2] * t - to_center[2],
        ];
        let u = dot3(delta, self.right) / self.half_w;
        let v = dot3(delta, self.up) / self.half_h;
        Some(((u + 1.0) * 0.5 * self.gui_w, (1.0 - v) * 0.5 * self.gui_h))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    Open,
    Closing,
}

/// Thread-safe runtime state shared between `mod_api::Hooks` and `[[native.mixin]]` helpers.
pub struct SpatialRuntime {
    pub config: SpatialConfig,
    phase: Phase,
    is_container_now: bool,
    saved_camera_type: Option<CameraType>,

    // Interpolated eye pose from tick snapshots + `partial`
    prev_eye_pos: [f64; 3],
    curr_eye_pos: [f64; 3],
    prev_eye_rot: [f32; 2],
    curr_eye_rot: [f32; 2],
    eye_pos: [f64; 3],
    eye_height: f32,
    eye_rot: [f32; 2], // [yaw, pitch]
    has_tick_pose: bool,
    fov_deg: f32,

    // Anchored orientation when the screen opened
    anchored_yaw: f32,
    anchored_pitch: f32,

    // Camera transition state
    transition_start: Option<Instant>,
    start_cam_pos: [f64; 3],
    start_cam_yaw: f32,
    start_cam_pitch: f32,
    last_cam_pos: [f64; 3],
    last_cam_offset: [f64; 3],
    last_cam_yaw: f32,
    last_cam_pitch: f32,
    cam_target_pos: [f64; 3],

    // Mouse parallax smoothing
    raw_cursor: (f32, f32),
    last_mapped_cursor: Option<(f32, f32)>,
    gui_size: (f32, f32),
    parallax_yaw: f32,
    parallax_pitch: f32,
    last_parallax: Option<Instant>,

    // Open animation & active world plane
    open_start: Option<Instant>,
    use_first_person_plane: bool,
    pub current_plane: Option<WorldPlane>,
}

impl Default for SpatialRuntime {
    fn default() -> Self {
        Self {
            config: SpatialConfig::default(),
            phase: Phase::Idle,
            is_container_now: false,
            saved_camera_type: None,
            prev_eye_pos: [0.0; 3],
            curr_eye_pos: [0.0; 3],
            prev_eye_rot: [0.0, 0.0],
            curr_eye_rot: [0.0, 0.0],
            eye_pos: [0.0; 3],
            eye_height: 1.62,
            eye_rot: [0.0, 0.0],
            has_tick_pose: false,
            fov_deg: 70.0,
            anchored_yaw: 0.0,
            anchored_pitch: 0.0,
            transition_start: None,
            start_cam_pos: [0.0; 3],
            start_cam_yaw: 0.0,
            start_cam_pitch: 0.0,
            last_cam_pos: [0.0; 3],
            last_cam_offset: [0.0; 3],
            last_cam_yaw: 0.0,
            last_cam_pitch: 0.0,
            cam_target_pos: [0.0; 3],
            raw_cursor: (0.0, 0.0),
            last_mapped_cursor: None,
            gui_size: (480.0, 270.0),
            parallax_yaw: 0.0,
            parallax_pitch: 0.0,
            last_parallax: None,
            open_start: None,
            use_first_person_plane: false,
            current_plane: None,
        }
    }
}

#[inline]
fn same_point(a: (f32, f32), b: (f32, f32)) -> bool {
    (a.0 - b.0).abs() <= 1e-3 && (a.1 - b.1).abs() <= 1e-3
}

impl SpatialRuntime {
    pub fn is_active(&self) -> bool {
        self.phase == Phase::Open && self.config.enabled
    }

    pub fn is_camera_overridden(&self) -> bool {
        self.phase != Phase::Idle && self.config.enabled
    }

    /// Records tick-end player eye position/rotation for smooth `partial` interpolation.
    pub fn record_tick_pose(&mut self, eye_pos: [f64; 3], eye_height: f32, eye_rot: [f32; 2]) {
        if !self.has_tick_pose {
            self.prev_eye_pos = eye_pos;
            self.prev_eye_rot = eye_rot;
            self.has_tick_pose = true;
        } else {
            self.prev_eye_pos = self.curr_eye_pos;
            self.prev_eye_rot = self.curr_eye_rot;
        }
        self.curr_eye_pos = eye_pos;
        self.curr_eye_rot = eye_rot;
        self.eye_height = eye_height;
        self.eye_pos = eye_pos;
        self.eye_rot = eye_rot;
    }

    pub fn set_container_open(&mut self, open: bool) {
        self.is_container_now = open;
    }

    /// Directly sets eye pose (primarily used by unit tests).
    pub fn record_camera_view(
        &mut self,
        is_container: bool,
        eye_pos: [f64; 3],
        eye_height: f32,
        eye_rot: [f32; 2],
    ) {
        self.is_container_now = is_container;
        self.prev_eye_pos = eye_pos;
        self.curr_eye_pos = eye_pos;
        self.prev_eye_rot = eye_rot;
        self.curr_eye_rot = eye_rot;
        self.has_tick_pose = true;
        self.eye_pos = eye_pos;
        self.eye_height = eye_height;
        self.eye_rot = eye_rot;
    }

    /// Computes camera pose override and target `CameraType` during `Hooks::camera_pose`.
    ///
    /// When `allow_position_override` is `false` (for example in multiplayer or when block collision
    /// cannot be verified), the camera position is kept at the player's eye (`FirstPerson`) so the
    /// camera never clips behind blocks after the host's collision check.
    pub fn compute_camera_pose<F>(
        &mut self,
        partial: f32,
        vanilla_pos: [f64; 3],
        vanilla_yaw: f32,
        vanilla_pitch: f32,
        current_cam_type: CameraType,
        fov_deg: f32,
        allow_position_override: bool,
        clip_camera: F,
    ) -> Option<([f64; 3], f32, f32, CameraType)>
    where
        F: FnOnce([f64; 3], [f64; 3]) -> [f64; 3],
    {
        if self.has_tick_pose {
            let p = partial.clamp(0.0, 1.0);
            self.eye_pos = [
                lerp_f64(self.prev_eye_pos[0], self.curr_eye_pos[0], p),
                lerp_f64(self.prev_eye_pos[1], self.curr_eye_pos[1], p),
                lerp_f64(self.prev_eye_pos[2], self.curr_eye_pos[2], p),
            ];
            self.eye_rot = [
                rot_lerp(self.prev_eye_rot[0], self.curr_eye_rot[0], p),
                lerp_f32(self.prev_eye_rot[1], self.curr_eye_rot[1], p),
            ];
        } else if current_cam_type == CameraType::FirstPerson {
            self.eye_pos = vanilla_pos;
            self.eye_rot = [vanilla_yaw, vanilla_pitch];
        }
        if fov_deg.is_finite() && fov_deg > 10.0 {
            self.fov_deg = fov_deg;
        }
        let now = Instant::now();
        let active_open = self.config.enabled && self.is_container_now;
        let use_fp = self.config.view_mode == ViewMode::FirstPerson || !allow_position_override;
        self.use_first_person_plane = use_fp;

        if active_open {
            if self.phase != Phase::Open {
                if self.phase == Phase::Idle {
                    self.saved_camera_type = Some(current_cam_type);
                    self.anchored_yaw = self.eye_rot[0];
                    self.anchored_pitch = self.eye_rot[1];
                    self.start_cam_pos = vanilla_pos;
                    self.start_cam_yaw = vanilla_yaw;
                    self.start_cam_pitch = vanilla_pitch;
                    self.open_start = Some(now);
                } else {
                    // Resuming from closing transition
                    self.start_cam_pos = self.last_cam_pos;
                    self.start_cam_yaw = self.last_cam_yaw;
                    self.start_cam_pitch = self.last_cam_pitch;
                    self.open_start = Some(now);
                }
                self.transition_start = Some(now);
                self.phase = Phase::Open;
            }

            let dt = self
                .last_parallax
                .map(|p| now.saturating_duration_since(p).as_secs_f32().clamp(0.0, 0.1))
                .unwrap_or(1.0 / 60.0);
            self.last_parallax = Some(now);
            let p_alpha = 1.0 - (-dt / 0.05).exp();

            let norm_x = ((self.raw_cursor.0 / self.gui_size.0.max(1.0)) * 2.0 - 1.0).clamp(-1.0, 1.0);
            let norm_y = ((self.raw_cursor.1 / self.gui_size.1.max(1.0)) * 2.0 - 1.0).clamp(-1.0, 1.0);

            if use_fp {
                let target_py = norm_x * 90.0 * 0.08;
                let target_pp = norm_y * 180.0 * 0.04;
                self.parallax_yaw += (target_py - self.parallax_yaw) * p_alpha;
                self.parallax_pitch += (target_pp - self.parallax_pitch) * p_alpha;

                let yaw = self.anchored_yaw + self.parallax_yaw;
                let pitch = (self.anchored_pitch + self.parallax_pitch).clamp(-89.0, 89.0);
                let pos = vanilla_pos;
                self.cam_target_pos = self.eye_pos;
                self.last_cam_pos = pos;
                self.last_cam_offset = [0.0, 0.0, 0.0];
                self.last_cam_yaw = yaw;
                self.last_cam_pitch = pitch;
                let desired = self.saved_camera_type.unwrap_or(CameraType::FirstPerson);
                return Some((pos, yaw, pitch, desired));
            }

            // Third-person over-the-shoulder view (faithful to CameraMixin.java:260-290)
            let target_py = norm_x * 90.0 * self.config.parallax_yaw;
            let target_pp = norm_y * 180.0 * self.config.parallax_pitch;
            self.parallax_yaw += (target_py - self.parallax_yaw) * p_alpha;
            self.parallax_pitch += (target_pp - self.parallax_pitch) * p_alpha;

            let pos_yaw = self.anchored_yaw + self.config.camera_orbit_yaw;
            let yaw_rad = pos_yaw.to_radians();
            let (sy, cy) = yaw_rad.sin_cos();
            let side = if self.config.mirror_third_person {
                -self.config.camera_side_offset
            } else {
                self.config.camera_side_offset
            };

            let feet = [
                self.eye_pos[0],
                self.eye_pos[1] - self.eye_height as f64,
                self.eye_pos[2],
            ];
            let target_pos = [
                feet[0] + (sy * self.config.camera_distance + cy * side) as f64,
                feet[1] + self.config.camera_height_offset as f64,
                feet[2] + (-cy * self.config.camera_distance + sy * side) as f64,
            ];
            let target_yaw = pos_yaw + self.parallax_yaw;
            let target_pitch = (self.config.camera_target_pitch + self.parallax_pitch).clamp(-89.0, 89.0);

            let dur_s = (self.config.transition_ms / 1000.0).max(0.01);
            let t = self
                .transition_start
                .map(|s| (now.saturating_duration_since(s).as_secs_f32() / dur_s).clamp(0.0, 1.0))
                .unwrap_or(1.0);
            let eased = ease_out_cubic(t);

            let unclipped_pos = [
                lerp_f64(self.start_cam_pos[0], target_pos[0], eased),
                lerp_f64(self.start_cam_pos[1], target_pos[1], eased),
                lerp_f64(self.start_cam_pos[2], target_pos[2], eased),
            ];
            let cam_pos = clip_camera(self.eye_pos, unclipped_pos);
            let cam_yaw = rot_lerp(self.start_cam_yaw, target_yaw, eased);
            let cam_pitch = lerp_f32(self.start_cam_pitch, target_pitch, eased);

            self.cam_target_pos = target_pos;
            self.last_cam_pos = cam_pos;
            self.last_cam_offset = [
                cam_pos[0] - feet[0],
                cam_pos[1] - feet[1],
                cam_pos[2] - feet[2],
            ];
            self.last_cam_yaw = cam_yaw;
            self.last_cam_pitch = cam_pitch;

            return Some((cam_pos, cam_yaw, cam_pitch, CameraType::ThirdPersonBack));
        }

        // Not open: handle smooth close swing back to saved CameraType
        if self.phase == Phase::Open {
            self.phase = Phase::Closing;
            self.transition_start = Some(now);
            self.current_plane = None;
        }

        if self.phase == Phase::Closing {
            if use_fp {
                let restore = self.saved_camera_type.take().unwrap_or(CameraType::FirstPerson);
                self.phase = Phase::Idle;
                return Some((vanilla_pos, vanilla_yaw, vanilla_pitch, restore));
            }
            let dur_s = ((self.config.transition_ms * 0.85) / 1000.0).max(0.01);
            let t = self
                .transition_start
                .map(|s| (now.saturating_duration_since(s).as_secs_f32() / dur_s).clamp(0.0, 1.0))
                .unwrap_or(1.0);
            let eased = ease_out_cubic(t);

            let feet = [
                self.eye_pos[0],
                self.eye_pos[1] - self.eye_height as f64,
                self.eye_pos[2],
            ];
            let from_pos = [
                feet[0] + self.last_cam_offset[0],
                feet[1] + self.last_cam_offset[1],
                feet[2] + self.last_cam_offset[2],
            ];
            let restore = self.saved_camera_type.unwrap_or(CameraType::FirstPerson);
            let target_pos = if restore == CameraType::FirstPerson {
                self.eye_pos
            } else {
                vanilla_pos
            };
            let target_yaw = if restore == CameraType::FirstPerson {
                self.eye_rot[0]
            } else {
                vanilla_yaw
            };
            let target_pitch = if restore == CameraType::FirstPerson {
                self.eye_rot[1]
            } else {
                vanilla_pitch
            };

            let unclipped_pos = [
                lerp_f64(from_pos[0], target_pos[0], eased),
                lerp_f64(from_pos[1], target_pos[1], eased),
                lerp_f64(from_pos[2], target_pos[2], eased),
            ];
            let pos = clip_camera(self.eye_pos, unclipped_pos);
            let yaw = rot_lerp(self.last_cam_yaw, target_yaw, eased);
            let pitch = lerp_f32(self.last_cam_pitch, target_pitch, eased);

            if t >= 1.0 {
                self.saved_camera_type = None;
                self.phase = Phase::Idle;
                return Some((target_pos, target_yaw, target_pitch, restore));
            }
            return Some((pos, yaw, pitch, CameraType::ThirdPersonBack));
        }

        None
    }

    /// Builds the frame's `WorldPlane` during `Game::hud` and maps `game.cursor`.
    pub fn step_hud(
        &mut self,
        is_container_screen: bool,
        gui_w: f32,
        gui_h: f32,
        current_cursor: (f32, f32),
    ) -> Option<(f32, f32)> {
        self.is_container_now = is_container_screen;
        self.gui_size = (gui_w.max(1.0), gui_h.max(1.0));
        if !self.config.enabled || !is_container_screen || self.phase != Phase::Open {
            self.current_plane = None;
            let had_mapped = self.last_mapped_cursor.take().is_some();
            if had_mapped {
                return Some(self.raw_cursor);
            }
            self.raw_cursor = current_cursor;
            return None;
        }

        if !self.last_mapped_cursor.is_some_and(|m| same_point(m, current_cursor)) {
            self.raw_cursor = current_cursor;
        }

        let plane = self.build_world_plane(gui_w, gui_h);
        self.current_plane = Some(plane);
        let mapped = plane.unproject(self.raw_cursor.0, self.raw_cursor.1);
        self.last_mapped_cursor = mapped;
        mapped
    }

    fn build_world_plane(&self, gui_w: f32, gui_h: f32) -> WorldPlane {
        let aspect = gui_w.max(1.0) / gui_h.max(1.0);
        let tan_half_fovy = (self.fov_deg.clamp(20.0, 120.0).to_radians() * 0.5).tan();

        let dur_s = (self.config.transition_ms / 1000.0).max(0.01);
        let open_t = self
            .open_start
            .map(|s| (Instant::now().saturating_duration_since(s).as_secs_f32() / dur_s).clamp(0.0, 1.0))
            .unwrap_or(1.0);
        let eased = ease_out_cubic(open_t);

        let (cam_q, cam_right, cam_up, cam_back) = view_basis(self.last_cam_yaw, self.last_cam_pitch);

        let is_fp = self.config.view_mode == ViewMode::FirstPerson || self.use_first_person_plane;
        let base_yaw = self.anchored_yaw;
        let yaw_rad = base_yaw.to_radians();
        let (sy, cy) = yaw_rad.sin_cos();

        let (center, plane_yaw_deg, plane_pitch_deg, world_scale) = if is_fp {
            let pr = self.anchored_pitch.to_radians();
            let (sp, cp) = pr.sin_cos();
            let look = [-sy * cp, -sp, cy * cp];
            let dist = self.config.fp_screen_distance;
            let c = [
                self.cam_target_pos[0] + (look[0] * dist) as f64,
                self.cam_target_pos[1] + (look[1] * dist) as f64,
                self.cam_target_pos[2] + (look[2] * dist) as f64,
            ];
            let scale = self.config.fp_screen_scale * lerp_f32(0.55, 1.0, eased);
            // Negate anchored_pitch because the plane faces back toward the camera (yaw + ~180°).
            (c, base_yaw + self.config.fp_screen_yaw_offset, -self.anchored_pitch, scale)
        } else {
            let side = if self.config.mirror_third_person {
                -self.config.screen_side_offset
            } else {
                self.config.screen_side_offset
            };
            let c = [
                self.cam_target_pos[0] + (-sy * self.config.screen_distance + cy * side) as f64,
                self.cam_target_pos[1] + self.config.screen_height_offset as f64,
                self.cam_target_pos[2] + (cy * self.config.screen_distance + sy * side) as f64,
            ];
            let scale = self.config.screen_scale * lerp_f32(0.55, 1.0, eased);
            (
                c,
                base_yaw + self.config.screen_yaw_offset,
                self.config.screen_pitch_offset,
                scale,
            )
        };

        // Screen local basis: at yaw=0, screen_yaw_offset=180°, facing back toward player (-Z):
        // right points West (-X), up points +Y, normal points South (+Z) or North (-Z).
        let q_screen = quat_yxz(
            std::f32::consts::PI - plane_yaw_deg.to_radians(),
            -plane_pitch_deg.to_radians(),
        );
        let right = quat_rotate(q_screen, [-1.0, 0.0, 0.0]);
        let up = quat_rotate(q_screen, [0.0, 1.0, 0.0]);
        let normal = quat_rotate(q_screen, [0.0, 0.0, 1.0]);

        WorldPlane {
            center,
            right,
            up,
            normal,
            half_w: aspect * 0.5 * world_scale,
            half_h: 0.5 * world_scale,
            cam_pos: self.last_cam_pos,
            cam_q,
            cam_right,
            cam_up,
            cam_back,
            tan_half_fovy,
            aspect,
            gui_w: gui_w.max(1.0),
            gui_h: gui_h.max(1.0),
        }
    }

    pub fn update_cursor(&mut self, raw_x: f32, raw_y: f32) -> (f32, f32) {
        if self.last_mapped_cursor.is_some_and(|m| same_point(m, (raw_x, raw_y))) {
            return (raw_x, raw_y);
        }
        self.raw_cursor = (raw_x, raw_y);
        if self.is_active()
            && let Some(plane) = self.current_plane
            && let Some(mapped) = plane.unproject(raw_x, raw_y)
        {
            self.last_mapped_cursor = Some(mapped);
            return mapped;
        }
        (raw_x, raw_y)
    }
}

static RUNTIME: OnceLock<Mutex<SpatialRuntime>> = OnceLock::new();

pub fn runtime() -> &'static Mutex<SpatialRuntime> {
    RUNTIME.get_or_init(|| Mutex::new(SpatialRuntime::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_disabled() {
        assert!(!SpatialConfig::default().enabled);
    }

    #[test]
    fn view_basis_matches_horizon_conventions() {
        let (_, right, up, back) = view_basis(0.0, 0.0);
        // Facing South (+Z): camera -Z = (0,0,+1) => back = (0,0,-1); right = West (-1,0,0)
        assert!((right[0] + 1.0).abs() < 1e-5);
        assert!((up[1] - 1.0).abs() < 1e-5);
        assert!((back[2] + 1.0).abs() < 1e-5);
    }

    #[test]
    fn first_person_plane_faces_camera_when_looking_down() {
        let mut rt = SpatialRuntime::default();
        rt.config.enabled = true;
        rt.config.view_mode = ViewMode::FirstPerson;
        rt.config.fp_screen_yaw_offset = 180.0;
        rt.raw_cursor = (240.0, 135.0);
        rt.record_camera_view(true, [0.0, 65.62, 0.0], 1.62, [25.0, 35.0]);
        rt.compute_camera_pose(
            1.0,
            [0.0, 65.62, 0.0],
            25.0,
            35.0,
            CameraType::FirstPerson,
            70.0,
            true,
            |_from, to| to,
        );
        rt.step_hud(true, 480.0, 270.0, (240.0, 135.0));
        let plane = rt.current_plane.expect("plane");
        // Camera forward is `-cam_back`; plane `up` and `right` must be orthogonal to it.
        let forward = [-plane.cam_back[0], -plane.cam_back[1], -plane.cam_back[2]];
        assert!(dot3(plane.up, forward).abs() < 1e-4);
        assert!(dot3(plane.right, forward).abs() < 1e-4);
    }

    #[test]
    fn multiplayer_keeps_camera_position_override_off() {
        let mut rt = SpatialRuntime::default();
        rt.config.enabled = true;
        rt.config.view_mode = ViewMode::ThirdPerson;
        let vanilla_pos = [10.0, 65.62, -20.0];
        rt.record_camera_view(true, vanilla_pos, 1.62, [0.0, 5.0]);
        let (pos, _, _, cam_type) = rt
            .compute_camera_pose(
                1.0,
                vanilla_pos,
                0.0,
                5.0,
                CameraType::FirstPerson,
                70.0,
                false,
                |_from, to| to,
            )
            .expect("override");
        assert_eq!(pos, vanilla_pos);
        assert_eq!(cam_type, CameraType::FirstPerson);
    }

    #[test]
    fn world_plane_orientation_and_roundtrip_across_player_yaws() {
        for yaw in [0.0_f32, 45.0, 135.0, -90.0] {
            let mut rt = SpatialRuntime::default();
            rt.config.enabled = true;
            rt.record_camera_view(true, [10.0, 65.62, -20.0], 1.62, [yaw, 5.0]);
            rt.raw_cursor = (300.0, 150.0);
            let (_, _, _, cam_type) = rt
                .compute_camera_pose(
                    1.0,
                    [10.0, 65.62, -20.0],
                    yaw,
                    5.0,
                    CameraType::FirstPerson,
                    70.0,
                    true,
                    |_from, to| to,
                )
                .expect("override");
            assert_eq!(cam_type, CameraType::ThirdPersonBack);

            rt.step_hud(true, 480.0, 270.0, (300.0, 150.0));
            let plane = rt.current_plane.expect("world plane");

            // Check orientation: left < right, top < bottom
            let left = plane.project(80.0, 135.0).expect("left");
            let right = plane.project(400.0, 135.0).expect("right");
            let top = plane.project(240.0, 40.0).expect("top");
            let bot = plane.project(240.0, 230.0).expect("bot");
            assert!(left.0 < right.0, "yaw={yaw}: left.x={} >= right.x={}", left.0, right.0);
            assert!(top.1 < bot.1, "yaw={yaw}: top.y={} >= bot.y={}", top.1, bot.1);

            // Round-trip projection <-> unprojection
            for gx in [100.0, 240.0, 380.0] {
                for gy in [50.0, 135.0, 220.0] {
                    let (sx, sy) = plane.project(gx, gy).expect("proj");
                    let (ux, uy) = plane.unproject(sx, sy).expect("unproj");
                    assert!(
                        (ux - gx).abs() < 1e-2 && (uy - gy).abs() < 1e-2,
                        "yaw={yaw} roundtrip ({gx},{gy}) -> ({sx},{sy}) -> ({ux},{uy})"
                    );
                }
            }
        }
    }
}
