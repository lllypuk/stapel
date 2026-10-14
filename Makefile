# cargo-binstall ставит typos сюда же, а не в PATH.
export PATH := $(HOME)/.cargo/bin:$(PATH)

GAMES := $(notdir $(wildcard games/*))

.PHONY: $(GAMES) test lint doc deps check cov mutants

# make pong; make pong NO_VSYNC=1
$(GAMES):
	RUST_LOG=$${RUST_LOG:-info} cargo run --release -p $@

test:
	cargo test --workspace

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

# app.rs и renderer.rs — склейка с окном, чистая логика поверхности — в surface.rs; порог только вверх.
cov:
	cargo llvm-cov --workspace --ignore-filename-regex 'engine/src/(app|renderer)\.rs' --fail-under-lines 98

# Исключения и причины — .cargo/mutants.toml.
mutants:
	cargo mutants -j 4
