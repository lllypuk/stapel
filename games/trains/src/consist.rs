use engine::{Handle, Model};

/// Вагонов за локомотивом не больше.
pub(crate) const MAX_CARS: usize = 5;
/// Зазор между кузовами соседних машин.
pub(crate) const GAP: f32 = 0.3;

/// Машина состава; перед — +Z модели.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Car {
    pub(crate) model: Handle<Model>,
    /// Габарит модели по z.
    pub(crate) length: f32,
    /// Отступы передней и задней тележки от переднего края.
    pub(crate) bogies: [f32; 2],
}

impl Car {
    /// `None` без габаритов или тележек: модель не загрузилась.
    pub(crate) fn new(handle: Handle<Model>, model: &Model) -> Option<Self> {
        let bounds = model.bounds()?;
        let z = |name: &'static str| {
            model
                .nodes
                .iter()
                .zip(&model.rest.globals)
                .filter(move |(n, _)| n.name == name)
                .map(|(_, g)| g.w_axis.z)
        };
        let front = z("wheels-front").next()?;
        let back = z("wheels-back").next().or_else(|| {
            let min = z("wheel").reduce(f32::min)?;
            let max = z("wheel").reduce(f32::max)?;
            Some(f32::midpoint(min, max))
        })?;
        Some(Self {
            model: handle,
            length: bounds.max.z - bounds.min.z,
            bogies: [bounds.max.z - front, bounds.max.z - back],
        })
    }
}

/// От переднего края первой машины до заднего края последней.
pub(crate) fn len(cars: &[Car]) -> f32 {
    cars.iter().map(|c| c.length).sum::<f32>() + GAP * cars.len().saturating_sub(1) as f32
}

/// Длина локомотива и `MAX_CARS` вагонов, если все — самые длинные из `catalog`.
pub(crate) fn max_len(catalog: &[Car]) -> f32 {
    let longest = catalog.iter().map(|c| c.length).fold(0.0, f32::max);
    longest * (MAX_CARS + 1) as f32 + GAP * MAX_CARS as f32
}

#[cfg(test)]
mod tests {
    use engine::Assets;

    use super::*;

    const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/train/");
    const CARS: [&str; 13] = [
        "train-locomotive-a",
        "train-diesel-a",
        "train-diesel-box-a",
        "train-electric-double-a",
        "train-electric-bullet-a",
        "train-carriage-box",
        "train-carriage-coal",
        "train-carriage-container-blue",
        "train-carriage-container-red",
        "train-carriage-flatbed-wood",
        "train-carriage-lumber",
        "train-carriage-tank",
        "train-carriage-wood",
    ];

    fn car(assets: &mut Assets, name: &str) -> Car {
        let h = assets.load_model(format!("{ASSETS}{name}.glb"));
        Car::new(h, assets.model(h)).unwrap_or_else(|| panic!("{name}"))
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn length_and_bogies_from_front_edge() {
        let mut assets = Assets::default();
        let table = [
            ("train-carriage-box", 2.7, [0.75, 1.95]),
            ("train-diesel-box-a", 2.6232, [0.7732, 1.8732]),
            ("train-locomotive-a", 2.6, [0.7, 1.6928]),
            ("train-electric-double-a", 2.5, [1.05, 1.85]),
        ];
        for (name, length, [front, back]) in table {
            let c = car(&mut assets, name);
            assert!(near(c.length, length), "{name}: {c:?}");
            assert!(
                near(c.bogies[0], front) && near(c.bogies[1], back),
                "{name}: {c:?}"
            );
        }
    }

    #[test]
    fn every_car_of_game_has_bogies_inside_body() {
        let mut assets = Assets::default();
        for name in CARS {
            let c = car(&mut assets, name);
            assert!(
                0.0 < c.bogies[0] && c.bogies[0] < c.bogies[1] && c.bogies[1] < c.length,
                "{name}: {c:?}"
            );
        }
    }

    #[test]
    fn no_car_without_bounds_or_front_bogie() {
        let mut assets = Assets::default();
        let h = assets.load_model(format!("{ASSETS}railroad-straight.glb"));
        assert_eq!(Car::new(h, assets.model(h)), None);
        assert_eq!(Car::new(h, &Model::default()), None);
    }

    #[test]
    fn len_adds_gap_between_cars() {
        let mut assets = Assets::default();
        let a = car(&mut assets, "train-diesel-a");
        let b = car(&mut assets, "train-carriage-tank");
        assert_eq!(len(&[]), 0.0);
        assert_eq!(len(&[a]), a.length);
        assert!(near(len(&[a, b, b]), a.length + 2.0 * b.length + 2.0 * GAP));
    }

    #[test]
    fn max_len_grows_with_max_cars_of_longest() {
        let mut assets = Assets::default();
        let catalog: Vec<_> = CARS.iter().map(|n| car(&mut assets, n)).collect();
        let longest = car(&mut assets, "train-electric-bullet-a");
        assert!(catalog.iter().all(|c| c.length <= longest.length));
        assert!(near(max_len(&catalog), len(&[longest; MAX_CARS + 1])));
        let short = &catalog[..1];
        assert!(near(max_len(short), len(&[short[0]; MAX_CARS + 1])));
    }
}
