# Android 构建指南

## 环境准备

- **JDK 17 及以上**（本机实测 JDK 21；Kotlin/Java 编译目标仍为 17）。
- **Android SDK**：`compileSdk 35`，通过 `sdkmanager "platforms;android-35"` 安装；设置 `ANDROID_HOME`。
- **Android NDK r26 及以上**（实测 r28 可用），设置 `ANDROID_NDK_HOME` 指向 NDK 目录。
- **cargo-ndk**：`cargo install cargo-ndk`。
- **Rust Android targets**：
  ```bash
  rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
  ```

## 构建

```bash
bash scripts/build-android.sh release   # 默认 release；也可传 debug
```

脚本依次完成：用 cargo-ndk 编译 `lanmsg-ffi` 的三个 ABI（arm64-v8a、armeabi-v7a、x86_64）到 `android/app/src/main/jniLibs/`，生成 UniFFI Kotlin 绑定，再调用 Gradle 打包。

仅重新生成原生库与绑定：`bash scripts/build-android-core.sh [debug|release]`。

## 产物路径

- release：`android/app/build/outputs/apk/release/app-release.apk`
- debug：`android/app/build/outputs/apk/debug/app-debug.apk`

## 签名配置

1. 复制 `android/keystore.properties.example` 为 `android/keystore.properties`（已被 `.gitignore` 忽略，不要提交）。
2. 填写 `storeFile`（相对 `android/` 目录或绝对路径）、`storePassword`、`keyAlias`、`keyPassword`。
3. 没有 keystore 时，release 包自动使用 debug 签名，并在构建日志打印警告 `keystore.properties not found; release build is signed with the debug key`；这种包只适合测试，不可对外分发。

注意：签名不同的包不能覆盖安装，需要先卸载（会清空应用数据）。

## 常见问题

### aws-lc-rs 编译失败

部分 NDK 或 target 组合下，依赖 `rustls` 的 `aws-lc-rs` 后端（需要 C 编译器与 CMake）可能编译失败。切换办法：让 `rustls` 使用纯 Rust 的 `ring` 后端（在对应 crate 的 `Cargo.toml` 中关闭默认特性，启用 `ring`），然后重新构建。先确认 `ANDROID_NDK_HOME` 指向 r26 及以上的 NDK、`cargo-ndk` 版本较新，再考虑切换。

### UnsatisfiedLinkError

1. 确认 `android/app/src/main/jniLibs/` 下 `arm64-v8a`、`armeabi-v7a`、`x86_64` 三个目录都有 `liblanmsg_ffi.so`。
2. 用 `unzip -l app-release.apk | grep '\.so'` 确认 APK 内同样包含三种 ABI 的 `liblanmsg_ffi.so` 与 JNA 的 `libjnidispatch.so`。
3. 确认设备 ABI 在上述三者之内（`adb shell getprop ro.product.cpu.abi`）。
4. release 包还要确认混淆未剔除 JNA 与 `uniffi.**`，规则见 `android/app/proguard-rules.pro`。

### Windows 上构建

- 在 Git Bash 或 WSL 中执行 `scripts/*.sh`（脚本依赖 bash、sed、python3）。
- 路径中避免空格与非 ASCII 字符；`ANDROID_NDK_HOME` 使用正斜杠或 Git Bash 风格路径。
- Gradle 使用 `gradlew.bat`；`keystore.properties` 中的 `storeFile` 路径用正斜杠。
