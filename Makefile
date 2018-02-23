TARGET_LINUX = x86_64-unknown-linux-gnu
TARGET_MACOS = x86_64-apple-darwin
TARGET_ARMV7 = arm-unknown-linux-gnueabihf

linux:
	rustup toolchain list | grep -q $(TARGET_LINUX) || rustup target add $(TARGET_LINUX)
	cargo build --release --target=$(TARGET_LINUX)

macos:
	rustup toolchain list | grep -q $(TARGET_MACOS) || rustup target add $(TARGET_MACOS)
	cargo build --release --target=$(TARGET_MACOS)

armv7:
	rustup toolchain list | grep -q $(TARGET_ARMV7) || rustup target add $(TARGET_ARMV7)
	cargo build --release --target=$(TARGET_ARMV7)
