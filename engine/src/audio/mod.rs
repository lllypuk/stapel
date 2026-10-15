pub(crate) mod decode;

use std::sync::Arc;

/// Стерео-интерлив f32 в частоте `Assets`; пустой — заглушка ошибки загрузки.
#[derive(Clone, Debug)]
pub struct Sound {
    #[cfg_attr(not(test), expect(dead_code, reason = "читает микшер, Task 6"))]
    pub(crate) frames: Arc<[f32]>,
}
