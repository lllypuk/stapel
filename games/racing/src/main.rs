mod layout;
mod pieces;
mod track;

use std::fs;

use engine::{
    Assets, Camera3D, Color, Config, Context, DirectionalLight, Fog, Game, Scene, Vec3, vec3,
};

use crate::layout::{Layout, Models};
use crate::pieces::Rot;
use crate::track::Track;

const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/");
const TRACK: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/tracks/oval.toml");

/// Камера обзора — над центром трассы, со сдвигом к +Z.
const OVERVIEW: Vec3 = vec3(0.0, 40.0, 30.0);

/// Обзор сверху на центр клеток дороги.
fn overview(track: &Track) -> Camera3D {
    let centers: Vec<Vec3> = (track.road.iter())
        .flat_map(|r| {
            let frame = r.frame();
            r.piece
                .cells(Rot::default())
                .map(move |c| frame.transform_point3(vec3(c.x as f32 + 0.5, 0.0, c.y as f32 + 0.5)))
        })
        .collect();
    let center = centers.iter().sum::<Vec3>() / centers.len().max(1) as f32;
    let mut camera = Camera3D {
        far: 400.0,
        ..Camera3D::default()
    };
    camera.look_at(center + OVERVIEW, center);
    camera
}

struct Racing {
    layout: Layout,
    camera: Camera3D,
    sky: Color,
}

impl Racing {
    fn new(track: &Track, assets: &mut Assets) -> Self {
        let models = Models::load(assets);
        Self {
            layout: Layout::new(track, &models, assets),
            camera: overview(track),
            sky: Color::from_srgb8(170, 205, 235),
        }
    }
}

impl Game for Racing {
    fn init(ctx: &mut Context) -> Self {
        let text = fs::read_to_string(TRACK).unwrap_or_else(|e| panic!("{TRACK}: {e}"));
        let track = Track::parse(&text).unwrap_or_else(|e| panic!("{TRACK}: {e}"));
        Self::new(&track, &mut ctx.assets)
    }

    fn update(&mut self, _ctx: &mut Context) {}

    fn draw(&mut self, ctx: &mut Context, _alpha: f32) {
        ctx.gfx.clear_color = self.sky;
        ctx.gfx.scene = Some(Scene {
            camera: self.camera,
            ambient: Color {
                r: 0.35,
                g: 0.37,
                b: 0.42,
                a: 1.0,
            },
            fog: Fog {
                color: self.sky,
                start: 80.0,
                end: 320.0,
            },
            sun: Some(DirectionalLight {
                dir: vec3(0.4, -1.0, 0.3),
                color: Color {
                    r: 1.0,
                    g: 0.95,
                    b: 0.85,
                    a: 1.0,
                },
            }),
            lights: Vec::new(),
            meshes: Vec::new(),
            models: self.layout.draws.clone(),
        });
    }
}

fn main() -> Result<(), engine::EventLoopError> {
    engine::run::<Racing>(Config {
        title: "racing".into(),
        size: (1280, 720),
        tick_hz: 60,
        vsync: std::env::var("NO_VSYNC").as_deref() != Ok("1"),
        pixel_perfect: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overview_looks_down_at_road_center() {
        let track = Track::parse(include_str!("testdata/oval.toml")).expect("овал");
        let camera = overview(&track);
        // Овал — клетки 0..7 × 0..5 по 4 м.
        let center = vec3(14.0, 0.0, 10.0);
        assert!(
            camera.pos.abs_diff_eq(center + OVERVIEW, 1e-4),
            "{}",
            camera.pos
        );
        let to_center = (center - camera.pos).normalize();
        assert!(camera.forward().abs_diff_eq(to_center, 1e-4));
    }

    #[test]
    fn draw_shows_track_lit_from_above_and_fogged_before_far_plane() {
        let mut ctx = Context::default();
        let track = Track::parse(include_str!("testdata/oval.toml")).expect("овал");
        let mut racing = Racing::new(&track, &mut ctx.assets);
        racing.draw(&mut ctx, 0.0);
        let scene = ctx.gfx.scene.expect("сцена");
        assert_ne!(scene.models.len(), 0);
        assert_eq!(scene.models, racing.layout.draws);
        assert!(scene.sun.is_some_and(|s| s.dir.y < 0.0));
        assert!(scene.fog.end <= scene.camera.far);
        assert_eq!(ctx.gfx.clear_color, scene.fog.color);
    }
}
