#!/usr/bin/env python3
"""在 Windows 上复刻 CI 的 APK 打包流程（无需 PowerShell 7）。

依赖：
  - D:/AndroidBuild/sdk 下的 build-tools/36.0.0、platforms/android-36
  - 已编译好的 target/aarch64-linux-android/release/libsportssimulation.so
  - javac / d8 / aapt2 / zipalign / apksigner

与 .github/workflows/apk.yml 的步骤一一对应。
"""
import os
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path

ROOT = Path(r"D:\Project\模拟程序")
SDK = Path(r"D:\AndroidBuild\sdk")
BT = SDK / "build-tools" / "36.0.0"
ANDROID_JAR = SDK / "platforms" / "android-36" / "android.jar"
NDK_BIN = SDK / "ndk" / "29.0.14206865" / "toolchains" / "llvm" / "prebuilt" / "windows-x86_64" / "bin"
TARGET = "aarch64-linux-android"
ABI = "arm64-v8a"
OUT = ROOT / "android" / "local-package"
SO = ROOT / "target" / TARGET / "release" / "libsportssimulation.so"
KEYSTORE = ROOT / "android" / ".signing" / "local-test.keystore"

VERSION_NAME = sys.argv[1] if len(sys.argv) > 1 else "0.5.1-android.2"
VERSION_CODE = sys.argv[2] if len(sys.argv) > 2 else "30502"


def run(cmd, **kw):
    print(f"  $ {' '.join(str(c) for c in cmd)[:150]}", flush=True)
    r = subprocess.run(cmd, capture_output=True, **kw)
    if r.returncode != 0:
        out = (r.stdout or b"").decode("utf-8", "replace")
        err = (r.stderr or b"").decode("utf-8", "replace")
        print(out[-2000:])
        print(err[-2000:], file=sys.stderr)
        raise SystemExit(f"命令失败: {cmd[0]}")


def main():
    if not SO.exists():
        raise SystemExit(f"缺少原生库: {SO}\n请先 cargo build --target {TARGET} --release --features android")

    if OUT.exists():
        shutil.rmtree(OUT)
    for d in ("classes", "dex", "staging/lib/" + ABI):
        (OUT / d).mkdir(parents=True, exist_ok=True)

    # 1) 复制源码并盖章 manifest
    shutil.copytree(ROOT / "android" / "java", OUT / "java")
    shutil.copytree(ROOT / "android" / "res", OUT / "res")
    manifest = (ROOT / "android" / "AndroidManifest.xml").read_text(encoding="utf-8")
    import re

    manifest = re.sub(r'android:versionCode="\d+"', f'android:versionCode="{VERSION_CODE}"', manifest)
    manifest = re.sub(r'android:versionName="[^"]*"', f'android:versionName="{VERSION_NAME}"', manifest)
    (OUT / "AndroidManifest.xml").write_text(manifest, encoding="utf-8")
    print(f"版本: {VERSION_NAME} (versionCode {VERSION_CODE})")

    # 2) javac → classes.jar → d8
    sources = [str(p) for p in (OUT / "java").rglob("*.java")]
    run([
        "javac", "-encoding", "UTF-8", "-source", "8", "-target", "8", "-Xlint:-options",
        "-bootclasspath", f"{ANDROID_JAR};{BT / 'core-lambda-stubs.jar'}",
        "-d", str(OUT / "classes"), *sources,
    ])
    run(["jar", "cf", str(OUT / "classes.jar"), "-C", str(OUT / "classes"), "."])
    run([str(BT / "d8.bat"), "--lib", str(ANDROID_JAR), "--min-api", "26",
         "--output", str(OUT / "dex"), str(OUT / "classes.jar")])

    # 3) aapt2 资源与链接
    # aapt2 无法处理含中文的绝对路径：切到 android/local-package 再执行相对路径
    aapt2 = str(BT / "aapt2.exe")
    run([aapt2, "compile", "--dir", "res", "-o", "resources.zip"], cwd=OUT)
    run([aapt2, "link", "-o", "unsigned.apk",
         "-I", str(ANDROID_JAR), "--manifest", "AndroidManifest.xml", "resources.zip"], cwd=OUT)

    # 4) 用 Python zipfile 注入 dex 与 .so（替代 CI 的 zip 命令）
    shutil.copy(SO, OUT / "staging" / "lib" / ABI / "libsportssimulation.so")
    shutil.copy(OUT / "dex" / "classes.dex", OUT / "staging" / "classes.dex")

    # 从 unsigned.apk 复制除 classes.dex / lib/ 外的所有条目
    tmp = OUT / "merged.apk"
    with zipfile.ZipFile(OUT / "unsigned.apk") as src, zipfile.ZipFile(tmp, "w", zipfile.ZIP_DEFLATED) as dst:
        for item in src.infolist():
            if item.filename == "classes.dex" or item.filename.startswith("lib/"):
                continue
            dst.writestr(item, src.read(item.filename))
        dst.write(OUT / "staging" / "classes.dex", "classes.dex")
        dst.write(OUT / "staging" / "lib" / ABI / "libsportssimulation.so",
                  f"lib/{ABI}/libsportssimulation.so")
    print(f"  合并完成: {tmp.stat().st_size / 1048576:.1f} MB")

    # 5) zipalign (16KB 页对齐)
    run([str(BT / "zipalign.exe"), "-f", "-P", "16", "4", str(tmp), str(OUT / "aligned.apk")])

    # 6) 签名
    if not KEYSTORE.exists():
        raise SystemExit(f"缺少签名密钥: {KEYSTORE}")
    signed = ROOT / "Sports-simulation-android-arm64.apk"
    run([str(BT / "apksigner.bat"), "sign",
         "--ks", str(KEYSTORE), "--ks-key-alias", "androiddebugkey",
         "--ks-pass", "pass:android", "--key-pass", "pass:android",
         "--out", str(signed), str(OUT / "aligned.apk")])
    run([str(BT / "apksigner.bat"), "verify", "--print-certs", str(signed)])

    print(f"\n完成: {signed}  ({signed.stat().st_size / 1048576:.1f} MB)")


if __name__ == "__main__":
    main()
