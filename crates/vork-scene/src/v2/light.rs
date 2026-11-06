use super::*;
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(extend("x-help-extra" = "lights: with no environment light the world is RGB [0.05,0.05,0.05]"))]
pub enum LightShape {
    Sun,
    Point,
    // Blender SpotLight: a point light limited to a cone.
    Spot,
    // Blender AreaLight.shape RECTANGLE.
    Rectangle,
    Ellipse,
    Environment,
}
impl LightShape {
    /// Blender 5.2.1 Add Light: sun irradiance 1 W/m², local power 10 W; Background strength 1.
    pub fn default_energy(&self) -> f32 {
        match self {
            Self::Sun => constants::SUN_INTENSITY,
            Self::Environment => constants::ENVIRONMENT_INTENSITY,
            _ => constants::LOCAL_INTENSITY,
        }
    }
    /// Colour when `color` is omitted: Blender lights are white; an environment
    /// without an image is Blender's factory world colour; an image's multiplier is white.
    pub fn default_color(&self, image: bool) -> [f32; 3] {
        if matches!(self, Self::Environment) && !image {
            constants::ENVIRONMENT_COLOR
        } else {
            constants::WHITE
        }
    }
}
/// Blender's spot blend is linear in the cosine range, not in angular width.
pub fn spot_inner_angle(size: f32, blend: f32) -> f32 {
    let outer_cos = (size * 0.5).cos();
    (outer_cos + (1. - outer_cos) * blend).clamp(-1., 1.).acos()
}
