use std::fmt;
use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};
use stapel::{Model, Vec3};

use crate::pieces::CARS;

/// Ведущие колёса.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Drive {
    Fwd,
    Rwd,
    Awd,
}

/// Характеристики машины из `cars/<модель>.toml`; единицы СИ, углы — градусы.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
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
    /// Угол руля на месте, градусы.
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

/// Как правка поля доходит до машины в заезде.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Apply {
    /// `Car::retune` на ходу.
    Live,
    /// Машина пересоздаётся на последнем чекпоинте.
    Restart,
}

/// Настраиваемое поле характеристик: слайдер панели и диапазон `validate`.
pub(crate) struct Tuning {
    /// Имя поля в TOML.
    pub(crate) label: &'static str,
    pub(crate) range: RangeInclusive<f32>,
    pub(crate) apply: Apply,
    pub(crate) field: fn(&mut CarConfig) -> &mut f32,
}

pub(crate) const TUNING: [Tuning; 12] = [
    Tuning {
        label: "mass",
        range: 300.0..=3000.0,
        apply: Apply::Live,
        field: |c| &mut c.mass,
    },
    Tuning {
        label: "power",
        range: 20_000.0..=400_000.0,
        apply: Apply::Live,
        field: |c| &mut c.power,
    },
    Tuning {
        label: "max_speed",
        range: 10.0..=80.0,
        apply: Apply::Live,
        field: |c| &mut c.max_speed,
    },
    Tuning {
        label: "brake",
        range: 2000.0..=30_000.0,
        apply: Apply::Live,
        field: |c| &mut c.brake,
    },
    Tuning {
        label: "grip_front",
        range: 0.5..=2.0,
        apply: Apply::Live,
        field: |c| &mut c.grip_front,
    },
    Tuning {
        label: "grip_rear",
        range: 0.5..=2.0,
        apply: Apply::Live,
        field: |c| &mut c.grip_rear,
    },
    Tuning {
        label: "steer_max",
        range: 10.0..=45.0,
        apply: Apply::Live,
        field: |c| &mut c.steer_max,
    },
    Tuning {
        label: "steer_at_speed",
        range: 0.1..=1.0,
        apply: Apply::Live,
        field: |c| &mut c.steer_at_speed,
    },
    Tuning {
        label: "spring",
        range: 10_000.0..=80_000.0,
        apply: Apply::Live,
        field: |c| &mut c.spring,
    },
    Tuning {
        label: "damper",
        range: 500.0..=8000.0,
        apply: Apply::Live,
        field: |c| &mut c.damper,
    },
    Tuning {
        label: "travel",
        range: 0.05..=0.4,
        apply: Apply::Restart,
        field: |c| &mut c.travel,
    },
    Tuning {
        label: "com_offset.y",
        range: -0.5..=0.3,
        apply: Apply::Restart,
        field: |c| &mut c.com_offset[1],
    },
];

impl CarConfig {
    pub(crate) fn parse(text: &str) -> Result<Self, CarError> {
        let car: Self = toml::from_str(text).map_err(|e| CarError::Toml(e.to_string()))?;
        car.validate()?;
        Ok(car)
    }

    /// Модель из `CARS`, поля `TUNING` в своих `range`, остальные числа конечны.
    pub(crate) fn validate(&self) -> Result<(), CarError> {
        if !CARS.contains(&self.model.as_str()) {
            return Err(CarError::Model {
                name: self.model.clone(),
            });
        }
        let mut car = self.clone();
        for t in &TUNING {
            let value = *(t.field)(&mut car);
            if !t.range.contains(&value) {
                return Err(CarError::Field {
                    field: t.label,
                    value,
                });
            }
        }
        let [x, _, z] = self.com_offset;
        if let Some(value) = [x, z].into_iter().find(|v| !v.is_finite()) {
            return Err(CarError::Field {
                field: "com_offset",
                value,
            });
        }
        Ok(())
    }

    /// Файл целиком, поля в порядке структуры.
    pub(crate) fn to_toml(&self) -> String {
        toml::to_string(self).expect("строки и конечные числа пишутся в TOML")
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

    use stapel::{Aabb3, Assets, Mat4, Node, Quat, vec3};

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
    fn parses_fields_with_steer_in_degrees() {
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
                steer_max: 30.0,
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
    fn com_offset_x_and_z_are_finite() {
        assert!(CarConfig::parse(&with("com_offset", "[-1.0, 0.0, 2.0]")).is_ok());
        for text in ["[inf, 0.0, 0.0]", "[0.0, 0.0, nan]"] {
            let err = CarConfig::parse(&with("com_offset", text));
            assert!(
                matches!(err, Err(CarError::Field { field: "com_offset", value }) if !value.is_finite()),
                "{text}: {err:?}"
            );
        }
    }

    #[test]
    fn every_car_file_survives_to_toml_byte_for_byte() {
        for name in CARS {
            let path = format!("{ASSETS}cars/{name}.toml");
            let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
            let car = CarConfig::parse(&text).unwrap_or_else(|e| panic!("{path}: {e}"));
            assert_eq!(car.to_toml(), text, "{path}");
            assert_eq!(CarConfig::parse(&car.to_toml()), Ok(car), "{path}");
        }
        let car = CarConfig::parse(RACE).expect("race");
        assert!(
            car.to_toml().contains("\nsteer_max = 30.0\n"),
            "{}",
            car.to_toml()
        );
    }

    #[test]
    fn tuning_edges_pass_validate_and_beyond_fail_with_label() {
        let base = CarConfig::parse(RACE).expect("race");
        assert_eq!(base.validate(), Ok(()));
        for t in &TUNING {
            let (lo, hi) = (*t.range.start(), *t.range.end());
            assert!(lo < hi, "{}", t.label);
            for (value, ok) in [
                (lo, true),
                (hi, true),
                (lo.next_down(), false),
                (hi.next_up(), false),
                (f32::NAN, false),
            ] {
                let mut car = base.clone();
                *(t.field)(&mut car) = value;
                match car.validate() {
                    Ok(()) => assert!(ok, "{} = {value}", t.label),
                    Err(CarError::Field { field, .. }) => {
                        assert!(!ok && field == t.label, "{} = {value}: {field}", t.label);
                    }
                    Err(e) => panic!("{} = {value}: {e}", t.label),
                }
            }
        }
    }

    #[test]
    fn tuning_label_names_its_toml_field() {
        for (i, t) in TUNING.iter().enumerate() {
            let mut car = CarConfig::parse(RACE).expect("race");
            let sentinel = 0.125 + i as f32;
            *(t.field)(&mut car) = sentinel;
            let table: toml::Table = toml::from_str(&car.to_toml()).expect("toml");
            let value = match t.label.strip_suffix(".y") {
                Some(name) => &table[name][1],
                None => &table[t.label],
            };
            assert_eq!(value.as_float(), Some(f64::from(sentinel)), "{}", t.label);
        }
    }

    #[test]
    fn only_travel_and_com_offset_restart_and_model_drive_are_not_tuned() {
        let restart: Vec<_> = (TUNING.iter())
            .filter(|t| t.apply == Apply::Restart)
            .map(|t| t.label)
            .collect();
        assert_eq!(restart, ["travel", "com_offset.y"]);
        // com_offset — одна строка, только Y.
        let table: toml::Table = toml::from_str(RACE).expect("toml");
        assert_eq!(TUNING.len(), table.len() - 2);
        assert!((TUNING.iter()).all(|t| t.label != "model" && t.label != "drive"));
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
