test-all:
	cargo test --all-features

install-fmt:
	rustup component add rustfmt

check-fmt:	install-fmt
	cargo fmt -- --check

install-clippy:
	rustup component add clippy

check-clippy:	install-clippy
	cargo clippy --all-features -- -D warnings

check-examples:
	(cd examples/client && cargo build)
	(cd examples/server && cargo build)
