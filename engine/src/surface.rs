use winit::dpi::PhysicalSize;

/// Размер для `configure`: wgpu паникует на нулевой стороне и на стороне больше лимита устройства.
pub(crate) fn surface_size(size: PhysicalSize<u32>, max: u32) -> Option<(u32, u32)> {
    (size.width > 0 && size.height > 0).then(|| (size.width.min(max), size.height.min(max)))
}

/// `Color` линейный, поэтому нужна `*Srgb`-поверхность: она сама кодирует в sRGB.
pub(crate) fn pick_format(formats: &[wgpu::TextureFormat]) -> Option<wgpu::TextureFormat> {
    formats.iter().copied().find(wgpu::TextureFormat::is_srgb)
}

/// Тот же выбор, что у `AutoVsync`/`AutoNoVsync`, но явный, чтобы в лог попал настоящий режим.
pub(crate) fn pick_present_mode(vsync: bool, modes: &[wgpu::PresentMode]) -> wgpu::PresentMode {
    use wgpu::PresentMode::{Fifo, FifoRelaxed, Immediate, Mailbox};
    let preferred: &[wgpu::PresentMode] = if vsync {
        &[FifoRelaxed, Fifo]
    } else {
        &[Immediate, Mailbox, Fifo]
    };
    preferred
        .iter()
        .copied()
        .find(|m| modes.contains(m))
        .unwrap_or(Fifo)
}

#[derive(Debug, PartialEq)]
pub(crate) enum Action {
    /// `reconfigure` — после `present`: пока текстура жива, `configure` паникует.
    Draw {
        reconfigure: bool,
    },
    Skip,
    Reconfigure,
    Recreate,
    Fail,
}

pub(crate) fn split(
    cur: wgpu::CurrentSurfaceTexture,
) -> (Option<wgpu::SurfaceTexture>, wgpu::SurfaceStatus) {
    use wgpu::{CurrentSurfaceTexture as C, SurfaceStatus as S};
    match cur {
        C::Success(t) => (Some(t), S::Good),
        C::Suboptimal(t) => (Some(t), S::Suboptimal),
        C::Timeout => (None, S::Timeout),
        C::Occluded => (None, S::Occluded),
        C::Outdated => (None, S::Outdated),
        C::Lost => (None, S::Lost),
        C::Validation => (None, S::Validation),
    }
}

pub(crate) fn action(status: &wgpu::SurfaceStatus) -> Action {
    use wgpu::SurfaceStatus as S;
    match status {
        S::Good => Action::Draw { reconfigure: false },
        S::Suboptimal => Action::Draw { reconfigure: true },
        S::Timeout | S::Occluded => Action::Skip,
        S::Outdated => Action::Reconfigure,
        S::Lost => Action::Recreate,
        S::Validation => Action::Fail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::PresentMode::*;
    use wgpu::SurfaceStatus as S;
    use wgpu::TextureFormat as F;

    #[test]
    fn zero_side_makes_surface_unavailable() {
        assert_eq!(
            surface_size(PhysicalSize::new(800, 600), 8192),
            Some((800, 600))
        );
        assert_eq!(surface_size(PhysicalSize::new(0, 600), 8192), None);
        assert_eq!(surface_size(PhysicalSize::new(800, 0), 8192), None);
        assert_eq!(surface_size(PhysicalSize::new(0, 0), 8192), None);
    }

    #[test]
    fn size_is_clamped_to_device_limit() {
        assert_eq!(
            surface_size(PhysicalSize::new(10240, 2880), 8192),
            Some((8192, 2880))
        );
        assert_eq!(
            surface_size(PhysicalSize::new(800, 9000), 8192),
            Some((800, 8192))
        );
    }

    #[test]
    fn split_without_frame_keeps_status() {
        use wgpu::CurrentSurfaceTexture as C;
        for (cur, status) in [
            (C::Timeout, S::Timeout),
            (C::Occluded, S::Occluded),
            (C::Outdated, S::Outdated),
            (C::Lost, S::Lost),
            (C::Validation, S::Validation),
        ] {
            let (frame, got) = split(cur);
            assert!(frame.is_none());
            assert_eq!(format!("{got:?}"), format!("{status:?}"));
        }
    }

    #[test]
    fn status_to_action() {
        assert_eq!(action(&S::Good), Action::Draw { reconfigure: false });
        assert_eq!(action(&S::Suboptimal), Action::Draw { reconfigure: true });
        assert_eq!(action(&S::Timeout), Action::Skip);
        assert_eq!(action(&S::Occluded), Action::Skip);
        assert_eq!(action(&S::Outdated), Action::Reconfigure);
        assert_eq!(action(&S::Lost), Action::Recreate);
        assert_eq!(action(&S::Validation), Action::Fail);
    }

    #[test]
    fn picks_only_srgb_format() {
        assert_eq!(
            pick_format(&[F::Bgra8Unorm, F::Bgra8UnormSrgb]),
            Some(F::Bgra8UnormSrgb)
        );
        assert_eq!(pick_format(&[F::Rgba16Float, F::Bgra8Unorm]), None);
        assert_eq!(pick_format(&[]), None);
    }

    #[test]
    fn present_mode_follows_vsync() {
        let all = [Fifo, FifoRelaxed, Immediate, Mailbox];
        assert_eq!(pick_present_mode(true, &all), FifoRelaxed);
        assert_eq!(pick_present_mode(true, &[Fifo, Immediate]), Fifo);
        assert_eq!(pick_present_mode(false, &all), Immediate);
        assert_eq!(pick_present_mode(false, &[Fifo, Mailbox]), Mailbox);
        assert_eq!(pick_present_mode(false, &[Fifo]), Fifo);
    }
}
