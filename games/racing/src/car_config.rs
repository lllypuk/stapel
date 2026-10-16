use std::fmt;

use engine::{Model, Vec3};
use serde::Deserialize;

use crate::pieces::CARS;

/// Ведущие колёса.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Drive {
    Fwd,
    Rwd,
    Awd,
}

/// Характеристики машины из `cars/<модель>.toml`; единицы СИ, углы в файле — градусы.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CarConfig {
    /// Имя модели в `assets/` без `.glb`.
    pub(crate) model: String,
    /// кг.
    pub(crate) mass: f32,
    /// Вт.
    pub(crate) power: f32,
    /// м/с.
    pub(crate) max_speed: f32,
    /// Тормозная сила на машину, Н.
    pub(crate) brake: f32,
    /// Множитель трения шин передней оси.
    pub(crate) grip_front: f32,
    /// Множитель трения шин задней оси.
    pub(crate) grip_rear: f32,
    /// Угол руля на месте; после `parse` — радианы.
    pub(crate) steer_max: f32,
    /// Доля `steer_max` на `max_speed`, (0, 1].
    pub(crate) steer_at_speed: f32,
    /// Жёсткость пружины колеса, Н/м.
    pub(crate) spring: f32,
    /// Демпфер колеса, Н·с/м.
    pub(crate) damper: f32,
    /// Ход подвески, м.
    pub(crate) travel: f32,
    /// Центр масс от центра габаритов модели, м.
    pub(crate) com_offset: [f32; 3],
    pub(crate) drive: Drive,
}

/// Ошибка характеристик машины.
#[derive(Debug, PartialEq)]
pub(crate) enum CarError {
    Toml(String),
    Model { name: String },
    Field { field: &'static str, value: f32 },
    Wheel { node: &'static str },
    WheelRadius { node: &'static str, radius: f32 },
}

impl fmt::Display for CarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Toml(msg) => f.write_str(msg),
            Self::Model { name } => write!(f, "модели {name:?} нет в pieces::CARS"),
            Self::Field { field, value } => write!(f, "{field} = {value} вне допустимых"),
            Self::Wheel { node } => write!(f, "у модели нет колеса {node} с мешем"),
            Self::WheelRadius { node, radius } => {
                write!(f, "радиус колеса {node} = {radius} вне допустимых")
            }
        }
    }
}

impl CarConfig {
    pub(crate) fn parse(text: &str) -> Result<Self, CarError> {
        let mut car: Self = toml::from_str(text).map_err(|e| CarError::Toml(e.to_string()))?;
        if !CARS.contains(&car.model.as_str()) {
            return Err(CarError::Model { name: car.model });
        }
        let positive = [
            ("mass", car.mass),
            ("power", car.power),
            ("max_speed", car.max_speed),
            ("brake", car.brake),
            ("grip_front", car.grip_front),
            ("grip_rear", car.grip_rear),
            ("steer_max", car.steer_max),
            ("steer_at_speed", car.steer_at_speed),
            ("spring", car.spring),
            ("damper", car.damper),
            ("travel", car.travel),
        ];
        if let Some(&(field, value)) = positive.iter().find(|(_, v)| !v.is_finite() || *v <= 0.0) {
            return Err(CarError::Field { field, value });
        }
        if car.steer_at_speed > 1.0 {
            return Err(CarError::Field {
                field: "steer_at_speed",
                value: car.steer_at_speed,
            });
        }
        if let Some(&value) = car.com_offset.iter().find(|v| !v.is_finite()) {
            return Err(CarError::Field {
                field: "com_offset",
                value,
            });
        }
        car.steer_max = car.steer_max.to_radians();
        Ok(car)
    }
}

/// Узлы колёс Car Kit в порядке `Wheels`.
const WHEELS: [&str; 4] = [
    "wheel-front-left",
    "wheel-front-right",
    "wheel-back-left",
    "wheel-back-right",
];

/// Колесо в позе покоя модели.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Wheel {
    /// Индекс узла в `Model::nodes`.
    pub(crate) node: usize,
    /// Центр колеса в пространстве модели — точка крепления подвески.
    pub(crate) mount: Vec3,
    pub(crate) radius: f32,
}

/// Перед-лево, перед-право, зад-лево, зад-право.
pub(crate) type Wheels = [Wheel; 4];

/// Колёса из габаритов узлов `wheel-*`; радиус — половина высоты габарита.
pub(crate) fn wheels(model: &Model) -> Result<Wheels, CarError> {
    let mut out = [Wheel {
        node: 0,
        mount: Vec3::ZERO,
        radius: 0.0,
    }; 4];
    for (wheel, name) in out.iter_mut().zip(WHEELS) {
        let found = (model.nodes.iter().enumerate())
            .find(|(_, n)| n.name == name)
            .and_then(|(i, n)| Some((i, n.bounds?.transformed(model.rest.globals[i]))));
        let (node, b) = found.ok_or(CarError::Wheel { node: name })?;
        let radius = (b.max.y - b.min.y) / 2.0;
        if !(radius.is_finite() && radius > 0.0) {
            return Err(CarError::WheelRadius { node: name, radius });
        }
        *wheel = Wheel {
            node,
            mount: (b.min + b.max) / 2.0,
            radius,
        };
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use engine::{Aabb3, Assets, Mat4, Node, Quat, vec3};

    use super::*;
    use crate::ASSETS;

    const RACE: &str = r#"
model = "race"
mass = 900.0
power = 150000.0
max_speed = 45.0
brake = 12000.0
grip_front = 1.2
grip_rear = 1.1
steer_max = 30.0
steer_at_speed = 0.25
spring = 30000.0
damper = 2500.0
travel = 0.15
com_offset = [0.0, -0.2, 0.1]
drive = "rwd"
"#;

    fn with(field: &str, value: &str) -> String {
        RACE.lines()
            .map(|l| {
                if l.starts_with(&format!("{field} =")) {
                    format!("{field} = {value}")
                } else {
                    l.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn parses_fields_and_steer_to_radians() {
        let car = CarConfig::parse(RACE).expect("race");
        assert_eq!(
            car,
            CarConfig {
                model: "race".into(),
                mass: 900.0,
                power: 150_000.0,
                max_speed: 45.0,
                brake: 12_000.0,
                grip_front: 1.2,
                grip_rear: 1.1,
                steer_max: 30f32.to_radians(),
                steer_at_speed: 0.25,
                spring: 30_000.0,
                damper: 2500.0,
                travel: 0.15,
                com_offset: [0.0, -0.2, 0.1],
                drive: Drive::Rwd,
            }
        );
        for (name, drive) in [("fwd", Drive::Fwd), ("awd", Drive::Awd)] {
            let car = CarConfig::parse(&with("drive", &format!("{name:?}"))).expect(name);
            assert_eq!(car.drive, drive);
        }
    }

    #[test]
    fn unknown_drive_and_field_are_errors() {
        for (text, needle) in [
            (with("drive", "\"4wd\""), "4wd"),
            (format!("{RACE}turbo = 1.0\n"), "turbo"),
        ] {
            match CarConfig::parse(&text) {
                Err(e @ CarError::Toml(_)) => assert!(e.to_string().contains(needle), "{e}"),
                other => panic!("{needle}: {other:?}"),
            }
        }
    }

    #[test]
    fn model_outside_cars_is_error() {
        let err = CarConfig::parse(&with("model", "\"suv\"")).expect_err("suv");
        assert_eq!(err.to_string(), "модели \"suv\" нет в pieces::CARS");
    }

    #[test]
    fn non_positive_or_non_finite_field_is_error() {
        let fields = [
            "mass",
            "power",
            "max_speed",
            "brake",
            "grip_front",
            "grip_rear",
            "steer_max",
            "steer_at_speed",
            "spring",
            "damper",
            "travel",
        ];
        for field in fields {
            for value in ["0.0", "-1.0", "nan", "inf"] {
                let err = CarConfig::parse(&with(field, value)).expect_err(field);
                assert!(
                    matches!(err, CarError::Field { field: f, .. } if f == field),
                    "{field} = {value}: {err:?}"
                );
            }
        }
        let err = CarConfig::parse(&with("mass", "-5.0")).expect_err("mass");
        assert_eq!(err.to_string(), "mass = -5 вне допустимых");
    }

    #[test]
    fn steer_at_speed_is_at_most_one() {
        assert!(CarConfig::parse(&with("steer_at_speed", "1.0")).is_ok());
        assert_eq!(
            CarConfig::parse(&with("steer_at_speed", "1.5")),
            Err(CarError::Field {
                field: "steer_at_speed",
                value: 1.5
            })
        );
    }

    #[test]
    fn com_offset_is_finite() {
        assert!(CarConfig::parse(&with("com_offset", "[-1.0, 0.0, 2.0]")).is_ok());
        let err = CarConfig::parse(&with("com_offset", "[0.0, inf, 0.0]"));
        assert!(
            matches!(err, Err(CarError::Field { field: "com_offset", value }) if value.is_infinite()),
            "{err:?}"
        );
    }

    fn assert_near(got: Vec3, want: Vec3) {
        assert!(got.abs_diff_eq(want, 1e-5), "{got} != {want}");
    }

    #[test]
    fn race_wheels_from_node_bounds() {
        let mut assets = Assets::default();
        let handle = assets.load_model(format!("{ASSETS}race.glb"));
        let model = assets.model(handle);
        let w = wheels(model).expect("колёса");
        for (wheel, name) in w.iter().zip(WHEELS) {
            assert_eq!(model.nodes[wheel.node].name, name);
            assert!(
                (wheel.radius - 0.3).abs() < 1e-5,
                "{name}: {}",
                wheel.radius
            );
        }
        let (front, back) = (0.639_886_2, -0.880_113_7);
        assert_near(w[0].mount, vec3(0.5, 0.3, front));
        assert_near(w[1].mount, vec3(-0.5, 0.3, front));
        assert_near(w[2].mount, vec3(0.5, 0.3, back));
        assert_near(w[3].mount, vec3(-0.5, 0.3, back));
    }

    #[test]
    fn every_car_file_parses_and_its_model_has_wheels_on_ground() {
        let mut assets = Assets::default();
        for name in CARS {
            let path = format!("{ASSETS}cars/{name}.toml");
            let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
            let car = CarConfig::parse(&text).unwrap_or_else(|e| panic!("{path}: {e}"));
            assert_eq!(car.model, name);
            let handle = assets.load_model(format!("{ASSETS}{name}.glb"));
            let model = assets.model(handle);
            let [fl, fr, bl, br] = wheels(model).unwrap_or_else(|e| panic!("{name}: {e}"));
            for w in [fl, fr, bl, br] {
                assert!((w.mount.y - w.radius).abs() < 1e-5, "{name}: {w:?}");
            }
            // Перёд — +Z, левый борт — +X.
            assert!(fl.mount.z > 0.0 && bl.mount.z < 0.0, "{name}");
            assert!(fl.mount.x > 0.0 && bl.mount.x > 0.0, "{name}");
            assert_near(fr.mount, fl.mount * vec3(-1.0, 1.0, 1.0));
            assert_near(br.mount, bl.mount * vec3(-1.0, 1.0, 1.0));
        }
    }

    fn wheel_node(name: &str, bounds: Option<Aabb3>) -> Node {
        Node {
            name: name.into(),
            parent: None,
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
            primitives: Vec::new(),
            skin: None,
            bounds,
        }
    }

    fn model(nodes: Vec<Node>) -> Model {
        let mut model = Model::default();
        model.rest.globals = vec![Mat4::IDENTITY; nodes.len()];
        model.nodes = nodes;
        model
    }

    #[test]
    fn flat_or_infinite_wheel_is_error() {
        // f32::MAX − (−f32::MAX) переполняется в +inf; inf в габарите дал бы NaN после преобразования.
        for (min_y, max_y, radius) in [(0.0, 0.0, 0.0), (-f32::MAX, f32::MAX, f32::INFINITY)] {
            let bounds = Some(Aabb3 {
                min: Vec3::splat(-0.3).with_y(min_y),
                max: Vec3::splat(0.3).with_y(max_y),
            });
            let nodes = WHEELS.iter().map(|n| wheel_node(n, bounds)).collect();
            let err = wheels(&model(nodes)).expect_err("радиус");
            assert_eq!(
                err,
                CarError::WheelRadius {
                    node: WHEELS[0],
                    radius
                }
            );
        }
        let err = CarError::WheelRadius {
            node: WHEELS[2],
            radius: 0.0,
        };
        assert_eq!(
            err.to_string(),
            "радиус колеса wheel-back-left = 0 вне допустимых"
        );
    }

    #[test]
    fn missing_or_meshless_wheel_is_error() {
        let cube = Some(Aabb3 {
            min: Vec3::splat(-0.3),
            max: Vec3::splat(0.3),
        });
        for (i, missing) in WHEELS.into_iter().enumerate() {
            let nodes = (WHEELS.iter())
                .filter(|&&n| n != missing)
                .map(|n| wheel_node(n, cube))
                .collect();
            assert_eq!(
                wheels(&model(nodes)),
                Err(CarError::Wheel { node: missing }),
                "{i}"
            );
        }
        let centered = wheels(&model(WHEELS.iter().map(|n| wheel_node(n, cube)).collect()));
        let want = [0, 1, 2, 3].map(|node| Wheel {
            node,
            mount: Vec3::ZERO,
            radius: 0.3,
        });
        assert_eq!(centered, Ok(want));
        let nodes = WHEELS
            .iter()
            .map(|&n| wheel_node(n, if n == WHEELS[1] { None } else { cube }))
            .collect();
        let err = wheels(&model(nodes)).expect_err("без меша");
        assert_eq!(err, CarError::Wheel { node: WHEELS[1] });
        assert_eq!(
            err.to_string(),
            "у модели нет колеса wheel-front-right с мешем"
        );
    }
}
