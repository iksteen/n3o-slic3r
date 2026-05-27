//! Object transform wrapper.
//!
//! Thin newtype over [`glam::Mat4`] with serde support + convenience
//! constructors. The scene state stores the *accumulated* world-space
//! transform for each object; transform ops compose new
//! deltas onto this. The renderer applies the matrix verbatim — no
//! transform math runs on the JS side.
//!
//! Serializes as a flat 16-element column-major `[f32; 16]` so the
//! renderer (Three.js's `Matrix4.fromArray`) can ingest the row of
//! the JSON payload without reshape work.

use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

/// World-space transform of a scene object.
///
/// `Transform::IDENTITY` is the default. `compose(other)` returns
/// `self * other` (apply `other` first, then `self`). The constructors
/// below are the common cases the transform-ops layer uses.
///
/// **Eventual home: `core/geometry/`** — same rationale as `Mesh`:
/// general affine transform, not scene-specific. `core/threemf`
/// imports it; future preview / plugin work will too. See
/// `core/scene/mod.rs` for the architectural review note.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Transform {
    /// 16 floats, **column-major** to match `glam::Mat4` and
    /// Three.js's `Matrix4`. Renderer side is one
    /// `new THREE.Matrix4().fromArray(json)`.
    pub matrix: [f32; 16],
}

impl Transform {
    pub const IDENTITY: Self = Self {
        matrix: [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0, //
        ],
    };

    pub fn from_mat4(m: Mat4) -> Self {
        Self {
            matrix: m.to_cols_array(),
        }
    }

    pub fn to_mat4(self) -> Mat4 {
        Mat4::from_cols_array(&self.matrix)
    }

    pub fn translation(v: Vec3) -> Self {
        Self::from_mat4(Mat4::from_translation(v))
    }

    pub fn scale(v: Vec3) -> Self {
        Self::from_mat4(Mat4::from_scale(v))
    }

    pub fn rotation(q: Quat) -> Self {
        Self::from_mat4(Mat4::from_quat(q))
    }

    /// Rotate `radians` around `axis` (axis is normalized internally).
    pub fn rotation_around(axis: Vec3, radians: f32) -> Self {
        Self::rotation(Quat::from_axis_angle(axis.normalize(), radians))
    }

    /// `self * other` — apply `other` first, then `self`. Useful for
    /// "translate, then rotate around the new center" composition.
    pub fn compose(self, other: Transform) -> Self {
        Self::from_mat4(self.to_mat4() * other.to_mat4())
    }

    /// Transform a point (treats `p` as a position; applies full
    /// translation + rotation + scale).
    pub fn apply_point(self, p: Vec3) -> Vec3 {
        self.to_mat4().transform_point3(p)
    }

    /// Transform a direction (ignores translation; rotates + scales).
    pub fn apply_vector(self, v: Vec3) -> Vec3 {
        self.to_mat4().transform_vector3(v)
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_neutral_for_compose() {
        let t = Transform::translation(Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(t.compose(Transform::IDENTITY), t);
        assert_eq!(Transform::IDENTITY.compose(t), t);
    }

    #[test]
    fn translation_moves_point() {
        let t = Transform::translation(Vec3::new(10.0, 0.0, 0.0));
        assert_eq!(t.apply_point(Vec3::ZERO), Vec3::new(10.0, 0.0, 0.0));
    }

    #[test]
    fn translation_then_translation_composes() {
        let a = Transform::translation(Vec3::new(1.0, 0.0, 0.0));
        let b = Transform::translation(Vec3::new(0.0, 2.0, 0.0));
        let composed = a.compose(b);
        assert_eq!(composed.apply_point(Vec3::ZERO), Vec3::new(1.0, 2.0, 0.0));
    }

    #[test]
    fn rotation_around_z_90_degrees_maps_x_to_y() {
        let t = Transform::rotation_around(Vec3::Z, std::f32::consts::FRAC_PI_2);
        let p = t.apply_point(Vec3::X);
        // Tolerate small float error from the matrix.
        assert!((p - Vec3::Y).length() < 1e-5, "got {p:?}");
    }

    #[test]
    fn translate_then_rotate_around_origin() {
        // Translate (1,0,0) then rotate 90° around Z. Point starts
        // at (0,0,0); after translate it's at (1,0,0); after rotate
        // around origin it's at (0,1,0).
        let t = Transform::rotation_around(Vec3::Z, std::f32::consts::FRAC_PI_2)
            .compose(Transform::translation(Vec3::X));
        let p = t.apply_point(Vec3::ZERO);
        assert!((p - Vec3::Y).length() < 1e-5, "got {p:?}");
    }

    #[test]
    fn serde_round_trip_via_json() {
        let t = Transform::translation(Vec3::new(1.0, 2.0, 3.0))
            .compose(Transform::rotation_around(Vec3::Y, 0.5));
        let json = serde_json::to_string(&t).unwrap();
        let parsed: Transform = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, t);
    }
}
