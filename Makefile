GAMES := $(notdir $(wildcard games/*))

.PHONY: $(GAMES) test lint check

# make pong; make pong NO_VSYNC=1
$(GAMES):
	RUST_LOG=$${RUST_LOG:-info} cargo run --release -p $@

test:
	cargo test --workspace

lint:
	cargo clippy --workspace --all-targets -- -D warnings

check:
	cargo fmt --all --check
	$(MAKE) lint test
