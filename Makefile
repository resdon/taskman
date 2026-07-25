.PHONY: all build release run clean install

# Default action when you just type "make"
all: build

# Build debug version
build:
	cargo build

# Build optimized release version
release:
	cargo build --release

# Build and run the project
run:
	cargo run

# Clean up cargo build artifacts
clean:
	cargo clean

# Build release and copy directly to your local bin path
install: release
	mkdir -p ~/.local/bin
	cp target/release/taskman ~/.local/bin/taskman
