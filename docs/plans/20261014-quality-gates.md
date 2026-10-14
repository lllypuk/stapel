# Проверки качества: линтеры, покрытие, мутанты, CI

## Overview
- Перед этапом 3 — автоматические ворота качества, чтобы рост движка не копил долг: строгий clippy (pedantic целиком), rustdoc, typos, неиспользуемые зависимости, лицензии и уязвимости, покрытие с порогом, мутационное тестирование.
- Одна цель `make check` локально и тот же набор в GitLab CI; офскрин-тест GPU в CI идёт на lavapipe (программный Vulkan) и не пропускается.
- Готово, когда: `make check` и пайплайн на `main` зелёные; нарушение каждого правила роняет проверку (проверено намеренной поломкой); выживших мутантов нет вне явных исключений с причиной.

## Context (from discovery)
- Сейчас: `cargo clippy -D warnings` в `make lint`, `cargo fmt --check` уже чист. `[workspace.lints]`, `clippy.toml`, `rust-toolchain.toml`, CI нет. `cargo` и `rustup` системные (`/usr/bin`), `~/.cargo/bin` нет.
- `clippy::pedantic` даёт ~120 предупреждений: касты `u32→f32` (32), `must_use_candidate` (25), касты с усечением и знаком (~15), `# Panics`/`# Errors` в доках (8), прочее по 1–7. Проверено plan-review на копии: синтаксис `[workspace.lints]` с `priority`, ключи `clippy.toml`, `unsafe_code = "forbid"` с `derive(Pod)` — рабочие.
- `unsafe` нет. `unwrap_used` в движке вне тестов — 0 (там `expect`), в Asteroids — есть. `#[allow(clippy::too_many_arguments)]` в `engine/src/frame.rs:78` упадёт на `allow_attributes`.
- `renderer.rs` смешанный: `impl Renderer` — склейка с окном, `action`, `pick_format`, `pick_present_mode`, `surface_size` — чистые функции с тестами.
- Офскрин-тест создаёт `Instance` через `new_without_display_handle_from_env` (`engine/src/sprite_pipeline.rs:293`): без `WGPU_BACKEND` включён и GL, адаптер найдётся через EGL/llvmpipe даже без Vulkan.
- GitLab: CI включён, раннер `home-server docker` (instance, `run_untagged`), registry `registry.gitlab.shatrov.tech/home/game-engine`. Локально docker 29.8. Rust 1.99.0, 5,2 тыс. строк.
- Инструменты не установлены: `cargo-binstall`, `cargo-llvm-cov`, `cargo-mutants`, `cargo-deny`, `cargo-machete`, `typos-cli`, компонент `llvm-tools`.

## Development Approach
- **testing approach**: каждая проверка вводится вместе с доказательством, что она ловит: временная поломка роняет проверку, затем откатывается. Код, меняющийся ради линтов, — под существующими тестами; новые тесты — там, где их требуют покрытие и мутанты.
- Следующая задача — только при зелёном `make check` в объёме, введённом к этому моменту.
- Исключение из правила — локальным `#[expect(lint, reason = "...")]`, а не `allow`: устаревшее исключение само станет ошибкой. Если линт срабатывает по-разному в test и не-test сборке (`unwrap_used`, `print_stderr` при `allow-*-in-tests`, `dead_code`) — `#[cfg_attr(not(test), expect(..))]`, иначе `--all-targets` даст `unfulfilled_lint_expectations`.
- Изменения скоупа — сразу в этот файл: ➕ новая задача, ⚠️ блокер.

## Solution Overview
- Версия компилятора закреплена `rust-toolchain.toml` — иначе обновление clippy на CI роняет сборку, которая локально зелёная.
- Линты — `[workspace.lints]`, крейты подключаются `[lints] workspace = true` по мере исправления: движок в Task 2, игры в Task 3. `missing_docs` — только движку (`#![deny(missing_docs)]` в `lib.rs`): у игр нет публичного API, а workspace-линты нельзя переопределить по крейту.
- `make check` — то же, что CI-стадия `lint` плюс тесты. Покрытие (`make cov`) в CI заменяет обычный прогон тестов: `cargo llvm-cov` сам их запускает, а GPU-тесты на lavapipe медленные. Мутанты — по расписанию.
- CI-образ собирается локально из `ci/Dockerfile` и пушится вручную; тег — переменная `CI_IMAGE` в `.gitlab-ci.yml`, поднимается вместе с правкой Dockerfile. Сборка образа в CI потребовала бы dind с `privileged`, а `rules:changes` пересобирал бы его в каждом пайплайне расписания.

## Technical Details

### Линты (`Cargo.toml`, `clippy.toml`)
- `[workspace.lints.rust]`: `unsafe_code = "forbid"`, `unreachable_pub = "deny"`, `unused_qualifications = "deny"`.
- `[workspace.lints.clippy]`: `all` и `pedantic` — `deny` с `priority = -1`. Поимённо `allow` с причиной в комментарии: `cast_precision_loss` (координаты и размеры в `f32` — норма графики), `must_use_candidate` (25 срабатываний без пользы для игры-потребителя). Остальное pedantic — в коде или локальным `#[expect]`.
- Из restriction — `deny`: `dbg_macro`, `todo`, `unimplemented`, `print_stdout`, `print_stderr` (логи — через `log`), `unwrap_used` (`expect` с сообщением-инвариантом разрешён), `allow_attributes`, `allow_attributes_without_reason`.
- `clippy.toml`: `allow-unwrap-in-tests = true`, `allow-print-in-tests = true`.

### Прочие проверки
- rustdoc: `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`; движок — `#![deny(missing_docs)]`.
- `typos` — без конфигурации: бинарные файлы и `.gitignore` он пропускает сам. `_typos.toml` — только при ложных срабатываниях реального прогона.
- `cargo machete` — без конфигурации.
- `cargo deny check` (`deny.toml`): `[graph] targets = ["x86_64-unknown-linux-gnu"]` (в `Cargo.lock` 23 крейта `windows*`); `[licenses]` — белый список из фактического дерева, `[licenses.private] ignore = true` и `publish = false` у всех крейтов workspace; `[advisories] unmaintained = "workspace"` (транзитивные unmaintained из wgpu/winit не исправить); `[bans] multiple-versions = "warn"`; `[sources] unknown-registry = "deny"`, `unknown-git = "deny"`. Скачивает advisory-db — `make check` требует сети, это принято.

### GPU-тест
- `REQUIRE_GPU=1` превращает пропуск офскрин-теста без адаптера в провал.
- `WGPU_BACKEND=vulkan` — иначе адаптер найдётся через GL и проверка lavapipe пройдёт ложно.
- В CI обе переменные — глобальные `variables:`: офскрин-тест гоняют и `coverage`, и `mutants`, пропуск там дал бы непонятное падение покрытия или missed-мутантов в `sprite_pipeline.rs`.

### Покрытие (`cargo-llvm-cov`)
- `cargo llvm-cov --workspace --ignore-filename-regex 'engine/src/(app|renderer)\.rs' --fail-under-lines <N>`. `app.rs` — склейка целиком; `renderer.rs` исключён из-за `impl Renderer` (склейка), хотя его чистые функции тестируются — llvm-cov режет только по файлам. Обе части проверяет запуск игры.
- `N` — замеренная база, вниз до целого; поднимается после Task 8. Только вверх: правка вниз — осознанное решение в коммите.
- Cobertura-отчёт и `coverage:`-регэксп для MR — когда появится работа через MR.

### Мутанты (`cargo-mutants`)
- `.cargo/mutants.toml`: `exclude_globs = ["engine/src/app.rs"]`; `exclude_re` — `"Renderer::"` (методы `impl Renderer`, чистые функции `renderer.rs` мутируются) и `"replace main with \\(\\)"` (`fn main` игр; глоб по `main.rs` выключил бы всю логику игр). Регэкспы сверить с `cargo mutants --list`. Каждое исключение — с причиной в комментарии.
- Ускорение — `-j N` и сужение скоупа. Каждый мутант `sprite_pipeline.rs` гоняет GPU-тесты на lavapipe: время прогона мерить в контейнере, не на локальном GPU.
- CI: job только по расписанию (раз в неделю), `missed > 0` — провал. Локально — `make mutants`.

### CI (`.gitlab-ci.yml`)
- `image: $CI_IMAGE`, `CI_IMAGE: $CI_REGISTRY_IMAGE/ci:1`. Глобальные `variables`: `REQUIRE_GPU=1`, `WGPU_BACKEND=vulkan`, `CARGO_HOME=$CI_PROJECT_DIR/.cargo-ci` (`cache:paths` только внутри проекта; не `.cargo` — там `mutants.toml`, cargo прочитал бы каталог как свой). Инструменты образа — в `/usr/local/cargo/bin`, в `PATH`.
- Стадии: `lint` (fmt, clippy, doc, typos, machete, deny) → `coverage`. `mutants` — `rules: if $CI_PIPELINE_SOURCE == "schedule"`.
- Кэш: `target/` и `.cargo-ci/registry`, `key: {files: [Cargo.lock, rust-toolchain.toml], prefix: $CI_IMAGE}`; `target/` с `llvm-cov-target` — гигабайты, поэтому `policy: pull` во всех job, кроме `coverage`.
- `ci/Dockerfile`: `rust:1.99.0-trixie` (mesa 25; в bookworm lavapipe 22.3 — риск, что wgpu 30 его не примет), `libvulkan1`, `mesa-vulkan-drivers`; `COPY rust-toolchain.toml` + `rustup show` — компоненты ровно из него; инструменты версий из Task 1.

## Implementation Steps

### Task 1: Закрепить toolchain, поставить инструменты, завести `make check`

**Files:**
- Create: `rust-toolchain.toml`
- Modify: `Makefile`

- [ ] `rust-toolchain.toml`: `1.99.0`, компоненты `clippy`, `rustfmt`, `llvm-tools`
- [ ] `cargo install cargo-binstall`, `~/.cargo/bin` в `PATH`; `cargo binstall cargo-llvm-cov cargo-mutants cargo-deny cargo-machete typos-cli`; версии записать в план
- [ ] `make check` = `cargo fmt --all --check` + существующие clippy и тесты
- [ ] проверка ворот: неотформатированная строка роняет `make check`
- [ ] `make check` — зелёный

### Task 2: Линты — конфигурация и движок

**Files:**
- Create: `clippy.toml`
- Modify: `Cargo.toml`, `engine/Cargo.toml`, `engine/src/**/*.rs`

- [ ] `[workspace.lints]` по Technical Details; `[lints] workspace = true` — пока только `engine`
- [ ] исправить срабатывания движка: касты с усечением и знаком — проверенными преобразованиями или `#[expect]` с причиной; `# Panics`/`# Errors` в доках; `frame.rs:78` `allow` → `expect` с причиной; длинные функции `sprite_pipeline` — разбить или `#[expect(too_many_lines, reason)]`
- [ ] существующие тесты не меняют ожиданий — поведение то же
- [ ] проверка ворот: `dbg!` и `.unwrap()` вне теста роняют clippy
- [ ] `make check` — зелёный

### Task 3: Линты — игры

**Files:**
- Modify: `games/*/Cargo.toml`, `games/asteroids/src/main.rs`, `games/pong/src/main.rs`, `games/clear-color/src/main.rs`

- [ ] `[lints] workspace = true` во всех играх
- [ ] исправить срабатывания: `unwrap` в Asteroids, касты, `bool as i32` → `i32::from`
- [ ] тесты игр без изменения ожиданий
- [ ] `make check` — зелёный

### Task 4: rustdoc и документация публичного API

**Files:**
- Modify: `engine/src/lib.rs`, `engine/src/**/*.rs`, `Makefile`

- [ ] `#![deny(missing_docs)]` в движке; одна строка на каждый публичный элемент без дока (правило лаконичности из `~/.claude/CLAUDE.md`)
- [ ] `make check` + `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`
- [ ] проверка ворот: битая внутридоковая ссылка и публичная функция без дока роняют `make check`
- [ ] `make check` — зелёный

### Task 5: typos, machete, deny

**Files:**
- Create: `deny.toml`
- Modify: `Makefile`, `engine/Cargo.toml`, `games/*/Cargo.toml`

- [ ] `typos`: прогнать, исправить найденное; `_typos.toml` — только при ложных срабатываниях
- [ ] `publish = false` во всех крейтах; `deny.toml` по Technical Details, белый список — из фактического дерева
- [ ] `make check` + `typos`, `cargo machete`, `cargo deny check`
- [ ] проверка ворот: опечатка в идентификаторе и неиспользуемая зависимость роняют `make check`
- [ ] `make check` — зелёный

### Task 6: Офскрин-тест не пропускается под `REQUIRE_GPU`

**Files:**
- Modify: `engine/src/sprite_pipeline.rs`

- [ ] без адаптера: `REQUIRE_GPU=1` → `panic!`, иначе `SKIPPED` как сейчас
- [ ] проверка: `WGPU_BACKEND=vulkan REQUIRE_GPU=1 VK_ICD_FILENAMES=/nonexistent cargo test -p engine sprite_pipeline` падает; без `REQUIRE_GPU` — пропускает (без `WGPU_BACKEND` адаптер нашёлся бы через GL)
- [ ] `make check` — зелёный

### Task 7: Покрытие с порогом

**Files:**
- Modify: `Makefile`, `.gitignore`

- [ ] `make cov` по Technical Details; замерить базу, записать в план, `N` — вниз до целого
- [ ] просмотреть непокрытое в чистых модулях: явная дыра — тест здесь же, остальное — в Task 8 через мутантов
- [ ] проверка ворот: при временном `N = база + 1` `make cov` падает
- [ ] `make cov` — зелёный

### Task 8: Мутанты и выжившие

**Files:**
- Create: `.cargo/mutants.toml`
- Modify: `Makefile`, `.gitignore` (`mutants.out*`), тестовые модули по результатам

- [ ] `.cargo/mutants.toml` по Technical Details, регэкспы сверены с `cargo mutants --list`; `make mutants`
- [ ] первый прогон, время записать в план
- [ ] каждый выживший — тест, который его убивает, или исключение с причиной; список исключений — в план
- [ ] повторный прогон: `missed = 0`
- [ ] проверка ворот: непокрытая тестом ветка в чистой функции даёт missed, затем откат
- [ ] перемерить покрытие, поднять `N` в `make cov`
- [ ] `make check`, `make cov` — зелёные

### Task 9: CI-образ

**Files:**
- Create: `ci/Dockerfile`

- [ ] `ci/Dockerfile` по Technical Details
- [ ] локально: `docker build`, в контейнере `make check` и `make cov` проходят, лог офскрин-теста показывает адаптер llvmpipe на Vulkan. Расхождение пикселей с эталоном — ⚠️ блокер, `TOL` не расширять
- [ ] в контейнере замерить `make mutants`; если > 60 мин — сузить скоуп, записать в план
- [ ] `docker login registry.gitlab.shatrov.tech`, push `ci:1`

### Task 10: `.gitlab-ci.yml`

**Files:**
- Create: `.gitlab-ci.yml`

- [ ] переменные, стадии и кэш по Technical Details
- [ ] расписание раз в неделю на `main` (`glab api -X POST projects/11/pipeline_schedules`)
- [ ] push ветки: пайплайн зелёный; намеренная поломка (clippy) в отдельном коммите роняет `lint`, затем откат
- [ ] ручной запуск расписания: `mutants` зелёный

### Task 11: Verify acceptance criteria
- [ ] `make check`, `make cov`, `make mutants` локально — зелёные
- [ ] пайплайн на ветке зелёный; в логе `coverage` офскрин-тест выполнен на llvmpipe через Vulkan, `SKIPPED` нет
- [ ] каждая проверка из Tasks 1–8 хотя бы раз уронила сборку намеренно (отмечено в задачах)

### Task 12: [Final] Update documentation
- [ ] `CLAUDE.md`, «Команды»: `make check` (нужна сеть — advisory-db), `make cov`, `make mutants`, `REQUIRE_GPU` и `WGPU_BACKEND`; «Правила»: исключения из линтов — `#[expect(.., reason)]` (`cfg_attr(not(test), ..)`, если линт зависит от тестовой сборки), порог покрытия только вверх, выживший мутант — тест или исключение с причиной, тег `CI_IMAGE` поднимается вместе с `ci/Dockerfile`
- [ ] `README.md`: установка инструментов
- [ ] `docs/design.md`, последняя строка раздела «Этапы»: как проверяется чистая логика (покрытие с порогом, мутанты)
- [ ] перенести план в `docs/plans/completed/`

## Post-Completion
- Раз в неделю смотреть результат расписания `mutants`; при росте времени прогона — сузить скоуп в `.cargo/mutants.toml`.
- Когда появится работа через MR: защитить `main` (merge только при зелёном пайплайне), Cobertura-отчёт в MR.
