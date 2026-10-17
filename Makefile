# cargo-binstall ставит typos сюда же, а не в PATH.
export PATH := $(HOME)/.cargo/bin:$(PATH)

GAMES := $(notdir $(wildcard games/*))

.PHONY: $(GAMES) test lint doc deps check cov mutants mutants-diff

# make pong; make pong NO_VSYNC=1
$(GAMES):
	RUST_LOG=$${RUST_LOG:-info} cargo run --release -p $@

# Под #[ignore] — долгие тесты, которые мутанты не гоняют.
test:
	cargo test --workspace -- --include-ignored

lint:
	cargo clippy --workspace --all-targets -- -D warnings

doc:
	RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

deps:
	cargo machete
	cargo deny check

check:
	cargo fmt --all --check
	typos
	$(MAKE) lint doc deps test

# app.rs, renderer.rs и audio/device.rs — склейка с окном и звуковой картой, чистая логика поверхности — в surface.rs; порог только вверх.
cov:
	cargo llvm-cov --workspace --ignore-filename-regex 'engine/src/(app|renderer|audio/device)\.rs' --fail-under-lines 98 -- --include-ignored

# Исключения и причины — .cargo/mutants.toml. Вполсилы: 8 сборок, 4×2 тестовых потока, nice — машина остаётся отзывчивой.
MUTANTS = RUST_TEST_THREADS=2 nice -n 19 cargo mutants -j 4 --jobserver-tasks 8
BASE ?= main

mutants:
	$(MUTANTS)

# Только строки ветки против BASE: тест, ослабленный в ветке, в нетронутом коде не заметит — тогда полный mutants.
mutants-diff:
	mkdir -p target && git diff $(BASE)...HEAD > target/branch.diff
	$(MUTANTS) --in-diff target/branch.diff
