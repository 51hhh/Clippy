use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

// 依存ライブラリの名前
const LIB_NAME: &str = "libvpx";
const LINK_NAME: &str = "vpx";
const SOURCE_ARCHIVE_URL: &str =
    "https://github.com/webmproject/libvpx/archive/refs/tags/v1.16.0.tar.gz";
const SOURCE_ARCHIVE_SHA256: &str =
    "7a479a3c66b9f5d5542a4c6a1b7d3768a983b1e5c14c60a9396edc9b649e015c";
const SOURCE_ARCHIVE_DIRECTORY: &str = "libvpx-1.16.0";

// シンボル書き換え用のプレフィックス
//
// prebuilt で配布する際、他のライブラリが同じ libvpx シンボル (vpx_codec_encode 等) を
// 使っていると衝突する。この定数のプレフィックスを付けることで回避する。
//
// 変換例:
//   vpx_codec_encode → shiguredo_vpx_codec_encode (vpx_ を shiguredo_vpx_ に置換)
//   vp8_denoiser_free → shiguredo_vpx_vp8_denoiser_free (内部シンボルは単純にプレフィックス付与)
const SYMBOL_PREFIX: &str = "shiguredo_vpx";

fn main() {
    // Cargo.toml か build.rs が更新されたら、依存ライブラリを再ビルドする
    println!("cargo::rerun-if-changed=Cargo.toml");
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=CARGO_FEATURE_SOURCE_BUILD");
    println!("cargo::rerun-if-env-changed=LIBVPX_TARGET");

    // 各種変数やビルドディレクトリのセットアップ
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("infallible"));
    let output_metadata_path = out_dir.join("metadata.rs");
    let output_bindings_path = out_dir.join("bindings.rs");

    // 各種メタデータを書き込む
    let (url, version) = get_url_and_version();
    fs::write(
        output_metadata_path,
        format!(
            concat!(
                "pub const BUILD_METADATA_REPOSITORY: &str={:?};\n",
                "pub const BUILD_METADATA_VERSION: &str={:?};\n",
            ),
            url, version
        ),
    )
    .expect("failed to write metadata file");

    if env::var("DOCS_RS").is_ok() {
        // Docs.rs 向けのビルドでは git clone ができないので build.rs の処理はスキップして、
        // 代わりに、ドキュメント生成時に最低限必要な定義だけをダミーで出力している。
        //
        // See also: https://docs.rs/about/builds
        fs::write(
            output_bindings_path,
            concat!(
                "pub struct vpx_codec_iface;",
                "pub struct vpx_codec_cx_pkt__bindgen_ty_1__bindgen_ty_1;",
                "pub struct vpx_codec_enc_cfg;",
                "pub struct vpx_image;",
                "pub struct vpx_codec_iter_t;",
                "pub struct vpx_codec_ctx;",
                "pub struct vpx_codec_err_t;",
            ),
        )
        .expect("write file error");
        return;
    }

    let output_lib_dir = if should_use_prebuilt() {
        download_prebuilt(&out_dir)
    } else {
        build_from_source(&out_dir, &output_bindings_path)
    };

    println!("cargo::rustc-link-search={}", output_lib_dir.display());
    println!("cargo::rustc-link-lib=static={LINK_NAME}");
}

// source-build feature が有効でなければ prebuilt を使う
fn should_use_prebuilt() -> bool {
    if env::var("CARGO_FEATURE_SOURCE_BUILD").is_ok() {
        return false;
    }
    true
}

// prebuilt バイナリをダウンロードして展開する
fn download_prebuilt(out_dir: &Path) -> PathBuf {
    let target = get_target_platform();
    let version = env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION is not set");
    let base_url = format!(
        "https://github.com/shiguredo/libvpx-rs/releases/download/{}",
        version
    );
    let archive_name = format!("libvpx-{}.tar.gz", target);
    let archive_url = format!("{}/{}", base_url, archive_name);
    let expected_sha256 = expected_prebuilt_sha256(&target);

    let archive_path = out_dir.join("prebuilt.tar.gz");
    let prebuilt_dir = out_dir.join("prebuilt");
    fs::create_dir_all(&prebuilt_dir).expect("failed to create prebuilt directory");

    // curl でアーカイブをダウンロード
    eprintln!("prebuilt ライブラリをダウンロード中: {}", archive_url);
    let status = Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(&archive_path)
        .arg(&archive_url)
        .status()
        .expect("failed to execute curl. Ensure curl is installed");
    if !status.success() {
        panic!("failed to download prebuilt library: {}", archive_url);
    }

    // 仓库固定的 SHA-256 是信任根；不能再从归档所在的同一 release 动态下载校验文件。
    verify_sha256(&archive_path, expected_sha256);

    // tar で展開
    let status = Command::new("tar")
        .args(["xzf"])
        .arg(&archive_path)
        .arg("-C")
        .arg(&prebuilt_dir)
        .status()
        .expect("failed to execute tar. Ensure tar is installed");
    if !status.success() {
        panic!("failed to extract prebuilt archive");
    }

    // ライブラリファイルを OUT_DIR/lib/ にコピー
    //
    // prebuilt アーカイブにはリリースビルド時にシンボル書き換え済みの libvpx.a と
    // #[link_name] 属性付きの bindings.rs が含まれているため、そのままコピーするだけでよい。
    let lib_dir = out_dir.join("lib");
    fs::create_dir_all(&lib_dir).expect("failed to create lib directory");
    fs::copy(
        prebuilt_dir.join("lib").join("libvpx.a"),
        lib_dir.join("libvpx.a"),
    )
    .expect("failed to copy libvpx.a");

    // bindings.rs を OUT_DIR/ にコピー
    fs::copy(
        prebuilt_dir.join("bindings.rs"),
        out_dir.join("bindings.rs"),
    )
    .expect("failed to copy bindings.rs");

    lib_dir
}

// SHA256 チェックサムを検証する
fn verify_sha256(file_path: &Path, expected: &str) {
    let actual = compute_sha256(file_path);
    if actual != expected {
        panic!(
            "SHA256 checksum mismatch:\n  expected: {}\n  actual:   {}",
            expected, actual
        );
    }
    eprintln!("SHA256 checksum verified: {}", actual);
}

fn expected_prebuilt_sha256(target: &str) -> &'static str {
    match target {
        "ubuntu-22.04_x86_64" => {
            "4c7a10b8d6f6e3331d55d8d4aaf2f6b942f6da52c0ecc1a8b946251536879c70"
        }
        "ubuntu-24.04_x86_64" => {
            "8175f600d2f44fa917fbcfd363c0e3ecf9427cd98c6d0ce97bd6b9ed71dc1141"
        }
        "ubuntu-26.04_x86_64" => {
            "fa42cae01b70b608c725a03fda03e6434d3f2597e93aa7210afd80ff16232c2b"
        }
        "windows_x86_64" => {
            "10c060d5c6d794ee74d227e8717e9237acf799c37d4309111839c80ce5f44eb9"
        }
        "macos_arm64" => {
            "9dd747b7d734ca9a6d44c2c861e32d5d041e1babc87b58f816b2ffb0ac66b90e"
        }
        _ => panic!(
            "Clippy has no pinned libvpx archive for target {target}; update the reviewed hash map first"
        ),
    }
}

// ファイルの SHA256 ハッシュを計算する
fn compute_sha256(path: &Path) -> String {
    let output = if cfg!(target_os = "macos") {
        // macOS: shasum を使用
        Command::new("shasum")
            .args(["-a", "256"])
            .arg(path)
            .output()
            .expect("failed to execute shasum. Ensure shasum is installed")
    } else if cfg!(target_os = "windows") {
        // Windows: certutil を使用
        Command::new("certutil")
            .args(["-hashfile"])
            .arg(path)
            .arg("SHA256")
            .output()
            .expect("failed to execute certutil")
    } else {
        // Linux: sha256sum を使用
        Command::new("sha256sum")
            .arg(path)
            .output()
            .expect("failed to execute sha256sum. Ensure coreutils is installed")
    };

    if !output.status.success() {
        panic!("failed to compute SHA256 checksum");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    if cfg!(target_os = "windows") {
        // certutil 出力形式:
        // SHA256 hash of <file>:
        // <hash>
        // CertUtil: -hashfile command completed successfully.
        stdout
            .lines()
            .nth(1)
            .expect("unexpected certutil output format")
            .trim()
            .to_lowercase()
    } else {
        // shasum / sha256sum 出力形式: <hash>  <filename>
        stdout
            .split_whitespace()
            .next()
            .expect("unexpected shasum/sha256sum output format")
            .to_lowercase()
    }
}

// ソースからビルドする
fn build_from_source(out_dir: &Path, output_bindings_path: &Path) -> PathBuf {
    let out_build_dir = out_dir.join("build/");
    let src_dir = out_build_dir.join(LIB_NAME);
    let input_header_dir = src_dir.join("include/vpx/");
    let output_lib_dir = src_dir.join("lib/");
    let _ = fs::remove_dir_all(&out_build_dir);
    fs::create_dir(&out_build_dir).expect("failed to create build directory");

    // タグは変更可能なので clone せず、仓库固定 SHA-256 の源码归档だけを使用する。
    download_source_archive(&out_build_dir);

    // ソースからビルドする
    build_from_source_platform(&src_dir);

    // 静的ライブラリのシンボルを書き換える
    let callbacks = rewrite_symbols(&output_lib_dir, out_dir);

    // バインディングを生成する
    //
    // parse_callbacks にシンボル書き換え用の ParseCallbacks を渡すことで、
    // 生成されるバインディングに #[link_name = "書き換え後のシンボル名"] が自動付与される。
    bindgen::Builder::default()
        .header(input_header_dir.join("vp8cx.h").display().to_string())
        .header(input_header_dir.join("vp8dx.h").display().to_string())
        .header(input_header_dir.join("vpx_codec.h").display().to_string())
        .header(input_header_dir.join("vpx_decoder.h").display().to_string())
        .header(input_header_dir.join("vpx_encoder.h").display().to_string())
        .parse_callbacks(Box::new(callbacks))
        .generate()
        .expect("failed to generate bindings")
        .write_to_file(output_bindings_path)
        .expect("failed to write bindings");

    output_lib_dir
}

// プラットフォームに応じてソースビルドの処理を分岐する
fn build_from_source_platform(src_dir: &Path) {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        build_from_source_windows(src_dir);
    } else {
        build_from_source_unix(src_dir);
    }
}

// Unix 環境でのソースビルド
fn build_from_source_unix(src_dir: &Path) {
    let success = Command::new("./configure")
        .arg("--disable-shared")
        .arg("--enable-vp9-highbitdepth")
        .arg(format!("--prefix={}", src_dir.display()))
        .current_dir(src_dir)
        .status()
        .is_ok_and(|status| status.success());
    if !success {
        panic!("[configure] failed to build {LIB_NAME}");
    }

    let success = Command::new("make")
        .current_dir(src_dir)
        .status()
        .is_ok_and(|status| status.success());
    if !success {
        panic!("[make] failed to build {LIB_NAME}");
    }

    let success = Command::new("make")
        .arg("install")
        .current_dir(src_dir)
        .status()
        .is_ok_and(|status| status.success());
    if !success {
        panic!("[make install] failed to build {LIB_NAME}");
    }
}

// Windows + MSYS2 環境でのソースビルド
fn build_from_source_windows(src_dir: &Path) {
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    match (target_arch.as_str(), target_env.as_str()) {
        ("x86_64", "msvc") => build_from_source_windows_msvc(src_dir),
        ("x86_64", "gnu") => build_from_source_windows_gnu(src_dir),
        _ => panic!(
            "unsupported Windows target for source-build: arch={}, env={}",
            target_arch, target_env
        ),
    }
}

fn build_from_source_windows_gnu(src_dir: &Path) {
    let configure_target = "x86_64-win64-gcc";

    // `configure` はシェルスクリプトなので sh 経由で実行する
    run_with_shell(
        src_dir,
        &format!(
            "./configure --target={configure_target} \
             --disable-shared --enable-vp9-highbitdepth --prefix=\"$(pwd)\""
        ),
        "configure",
    );
    run_with_shell(src_dir, "make", "make");
    run_with_shell(src_dir, "make install", "make install");
}

// Rust の Windows リリースは MSVC ABI を使う。上流の windows_x86_64 prebuilt は
// MinGW/pthread シンボルを含むため、固定ソースから Visual Studio project を生成してビルドする。
fn build_from_source_windows_msvc(src_dir: &Path) {
    patch_windows_msvc_project_generator(src_dir);
    run_with_shell(
        src_dir,
        "./configure --target=x86_64-win64-vs17 \
         --disable-shared --enable-vp9-highbitdepth \
         --disable-examples --disable-tools --disable-docs --disable-unit-tests \
         --enable-external-build --as=nasm",
        "configure (MSVC)",
    );
    run_with_shell(src_dir, "make vpx.vcxproj", "generate vpx.vcxproj");

    let project = src_dir.join("vpx.vcxproj");
    let status = Command::new("msbuild")
        .arg(&project)
        .args([
            "/m",
            "/p:Configuration=Release",
            "/p:Platform=x64",
            "/p:PreferredToolArchitecture=x64",
        ])
        .status()
        .expect("failed to execute msbuild. Ensure MSBuild is installed and on PATH");
    if !status.success() {
        panic!("[msbuild] failed to build {LIB_NAME} for Windows MSVC");
    }

    // libvpx の VS project は動的 CRT 用 static library を vpxmd.lib として出力する。
    // Rust 側の既存 #[link(name = \"vpx\")] を保つため、隔離した出力先で vpx.lib に正規化する。
    let built_library = src_dir.join("x64").join("Release").join("vpxmd.lib");
    let output_lib_dir = src_dir.join("lib");
    fs::create_dir_all(&output_lib_dir).expect("failed to create MSVC library output directory");
    fs::copy(&built_library, output_lib_dir.join("vpx.lib")).unwrap_or_else(|error| {
        panic!(
            "failed to normalize MSVC library {}: {error}",
            built_library.display()
        )
    });
}

// libvpx 1.16.0 は external-build 時にも GCC 用の -O3 を VS project generator に渡す。
// Microsoft vcpkg の同版向け修正と同じ 1 行だけを書き換え、固定源码が変わった場合は即座に失敗する。
fn patch_windows_msvc_project_generator(src_dir: &Path) {
    const ORIGINAL: &str = "        -*) die_unknown $opt\n";
    const PATCHED: &str =
        "        -*) : # Ignore unknown flags (e.g. -O3 leaked from GCC CFLAGS)\n";

    let script_path = src_dir.join("build/make/gen_msvs_vcxproj.sh");
    let script = fs::read_to_string(&script_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", script_path.display()));
    if script.matches(ORIGINAL).count() != 1 {
        panic!(
            "unexpected libvpx MSVC project generator at {}; refuse to apply an ambiguous patch",
            script_path.display()
        );
    }
    fs::write(&script_path, script.replacen(ORIGINAL, PATCHED, 1))
        .unwrap_or_else(|error| panic!("failed to patch {}: {error}", script_path.display()));
}

// shell 経由でコマンドを実行する
fn run_with_shell(src_dir: &Path, command: &str, step_name: &str) {
    let success = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(src_dir)
        .status()
        .is_ok_and(|status| status.success());
    if !success {
        panic!("[{}] failed to build {LIB_NAME} on Windows", step_name);
    }
}

// --- シンボル書き換え ---
//
// 他のライブラリとのシンボル衝突を回避するため、静的ライブラリ内の全シンボルに
// プレフィックスを付与する仕組み。
//
// llvm-nm / llvm-objcopy は rustup の llvm-tools コンポーネントに含まれるものを使用する。
// rust-toolchain.toml に components = ["llvm-tools"] の記載が必要。
//
// プラットフォームごとのシンボル形式の違い:
//   - macOS (Mach-O): シンボル先頭に `_` が付く (例: _vpx_codec_encode)
//   - Linux (ELF): 先頭 `_` なし (例: vpx_codec_encode)
//   - Windows x64 (COFF): 先頭 `_` なし (例: vpx_codec_encode)
//
// bindgen の generated_link_name_override は返した文字列に \u{1} プレフィックスを
// 自動付加する。\u{1} はコンパイラに「この名前をそのまま使え（マングリングするな）」と
// 指示するため、プラットフォーム固有のシンボル名（macOS なら _shiguredo_vpx_codec_encode）を
// そのまま返す必要がある。

/// llvm-nm / llvm-objcopy のパスを保持する
struct LlvmTools {
    nm: PathBuf,
    objcopy: PathBuf,
}

/// objcopy 用と bindgen 用の 2 つのリネームマップを保持する
///
/// 2 つのマップが必要な理由:
///   - objcopy_map: ライブラリ内の実シンボル名を書き換えるため、プラットフォーム依存の名前を使う
///   - bindgen_map: Rust コードからリンクする際の名前を指定するため、C シンボル名をキーにする
struct SymbolRenameMaps {
    /// llvm-objcopy の --redefine-syms 用マップ
    ///
    /// キー: 元のシンボル名 (例: macOS なら _vpx_codec_encode、Linux なら vpx_codec_encode)
    /// 値: 書き換え後のシンボル名 (例: macOS なら _shiguredo_vpx_codec_encode)
    objcopy_map: HashMap<String, String>,

    /// bindgen の #[link_name] 用マップ
    ///
    /// キー: C シンボル名 (プラットフォーム非依存、例: vpx_codec_encode)
    /// 値: 書き換え後のシンボル名 (プラットフォーム依存、例: macOS なら _shiguredo_vpx_codec_encode)
    ///
    /// bindgen は \u{1} プレフィックスを付加してマングリングを抑制するため、
    /// 値にはプラットフォーム固有のシンボル名を格納する必要がある。
    bindgen_map: HashMap<String, String>,
}

/// bindgen の ParseCallbacks 実装
///
/// バインディング生成時に、書き換え後のシンボル名を `#[link_name = "..."]` として付与する。
/// これにより lib.rs 側のコード変更なしでシンボル書き換えが透過的に動作する。
#[derive(Debug)]
struct SymbolLinkNameCallbacks {
    /// C シンボル名 → 書き換え後シンボル名のマップ
    rename_map: HashMap<String, String>,
}

impl bindgen::callbacks::ParseCallbacks for SymbolLinkNameCallbacks {
    /// bindgen がバインディングを生成する際に呼ばれるコールバック
    ///
    /// 戻り値が Some の場合、bindgen は #[link_name = "\u{1}<戻り値>"] を生成する。
    /// \u{1} プレフィックスによりコンパイラのシンボルマングリングが抑制されるため、
    /// 戻り値にはプラットフォーム固有のシンボル名を返す必要がある。
    fn generated_link_name_override(
        &self,
        item_info: bindgen::callbacks::ItemInfo<'_>,
    ) -> Option<String> {
        self.rename_map.get(item_info.name).cloned()
    }
}

/// 静的ライブラリのシンボルを書き換え、bindgen 用の ParseCallbacks を返す
///
/// 処理の流れ:
///   1. rustup の sysroot から llvm-nm / llvm-objcopy を探す
///   2. llvm-nm で静的ライブラリの定義済み外部シンボルを収集する
///   3. 収集したシンボルに対してリネームマップを生成する
///   4. マップファイルを書き出し、llvm-objcopy でライブラリ内のシンボルを書き換える
///   5. bindgen 用の ParseCallbacks を返す
fn rewrite_symbols(lib_dir: &Path, out_dir: &Path) -> SymbolLinkNameCallbacks {
    let tools = discover_llvm_tools();
    let lib_path = find_static_library(lib_dir);

    // macOS の Mach-O ではシンボル先頭に `_` が付くため、
    // プラットフォーム判定してリネームマップの生成時に考慮する
    let is_macos = env::var("CARGO_CFG_TARGET_OS")
        .map(|v| v == "macos")
        .unwrap_or(false);

    // シンボル名の変換ルール
    //
    // vpx_ プレフィックスを持つシンボル (公開 API) は vpx_ を SYMBOL_PREFIX_ に置換する。
    //   例: vpx_codec_encode → shiguredo_vpx_codec_encode
    //
    // それ以外のシンボル (vp8_*, vp9_* 等の内部シンボル) は先頭に SYMBOL_PREFIX_ を付与する。
    //   例: vp8_denoiser_free → shiguredo_vpx_vp8_denoiser_free
    let rename_symbol = |name: &str| -> Option<String> {
        if let Some(rest) = name.strip_prefix("vpx_") {
            Some(format!("{SYMBOL_PREFIX}_{rest}"))
        } else {
            Some(format!("{SYMBOL_PREFIX}_{name}"))
        }
    };

    // 全定義済み外部シンボルを収集してリネームマップを生成する
    let symbols = collect_defined_external_symbols(&tools.nm, &lib_path);
    let maps = build_symbol_rename_maps(&symbols, is_macos, &rename_symbol);

    // マップファイルを書き出してシンボルを書き換える
    let map_file = out_dir.join("symbol_rename_map.txt");
    write_objcopy_rename_map(&maps.objcopy_map, &map_file);
    rewrite_archive_symbols(&tools.objcopy, &lib_path, &map_file);

    SymbolLinkNameCallbacks {
        rename_map: maps.bindgen_map,
    }
}

/// 静的ライブラリのパスを探す
///
/// ビルド結果は Unix 系では libvpx.a、Windows では vpx.lib として出力される。
fn find_static_library(lib_dir: &Path) -> PathBuf {
    let unix_path = lib_dir.join("libvpx.a");
    if unix_path.exists() {
        return unix_path;
    }
    let win_path = lib_dir.join("vpx.lib");
    if win_path.exists() {
        return win_path;
    }
    panic!("static library not found in {}", lib_dir.display());
}

/// rustc --print sysroot の結果を取得する
///
/// llvm-tools は rustup が管理する sysroot 配下にインストールされるため、
/// sysroot のパスを取得して llvm-nm / llvm-objcopy の探索に使用する。
fn get_rustc_sysroot() -> PathBuf {
    let output = Command::new("rustc")
        .arg("--print")
        .arg("sysroot")
        .output()
        .expect("failed to run rustc --print sysroot");
    if !output.status.success() {
        panic!("rustc --print sysroot failed");
    }
    PathBuf::from(
        String::from_utf8(output.stdout)
            .expect("invalid UTF-8")
            .trim(),
    )
}

/// Windows 対応の実行ファイル名を生成する
///
/// Windows では実行ファイルに .exe 拡張子が必要。
fn exe_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// rustup の sysroot から llvm-nm / llvm-objcopy を探す
///
/// llvm-tools コンポーネントのバイナリは以下のパスに配置される:
///   <sysroot>/lib/rustlib/<host>/bin/llvm-nm
///   <sysroot>/lib/rustlib/<host>/bin/llvm-objcopy
///
/// rust-toolchain.toml に llvm-tools コンポーネントの記載が必要。
///
/// llvm-nm / llvm-objcopy はホスト上で実行するツールなので、クロスコンパイル時は
/// TARGET ではなく HOST のパスから探す必要がある。
/// 例: Windows CI でホストが x86_64-pc-windows-msvc、ターゲットが x86_64-pc-windows-gnu の場合、
/// llvm-tools は msvc 側にのみインストールされている。
fn discover_llvm_tools() -> LlvmTools {
    let sysroot = get_rustc_sysroot();
    // llvm-tools はホスト上で動作するため HOST を使う。
    // クロスコンパイル時に TARGET を使うと、ホスト側にインストールされた
    // llvm-tools が見つからない。
    let host = env::var("HOST").expect("HOST environment variable not set");
    let tools_dir = sysroot.join("lib/rustlib").join(host).join("bin");

    let nm = tools_dir.join(exe_name("llvm-nm"));
    let objcopy = tools_dir.join(exe_name("llvm-objcopy"));

    if !nm.exists() {
        panic!(
            "llvm-nm not found at {}. Run: rustup component add llvm-tools",
            nm.display()
        );
    }
    if !objcopy.exists() {
        panic!(
            "llvm-objcopy not found at {}. Run: rustup component add llvm-tools",
            objcopy.display()
        );
    }

    LlvmTools { nm, objcopy }
}

/// llvm-nm で静的ライブラリから定義済み外部シンボルを収集する
///
/// llvm-nm のオプション:
///   --defined-only: 定義済みシンボルのみ (未定義シンボルを除外)
///   --extern-only: 外部シンボルのみ (ローカルシンボルを除外)
///   --format=just-symbols: シンボル名のみ出力 (アドレスやタイプを省略)
///
/// 出力にはオブジェクトファイル名 (例: vp8_cx_iface.c.o:) も含まれるため、
/// is_c_identifier() でフィルタリングして純粋なシンボル名のみを抽出する。
fn collect_defined_external_symbols(nm_path: &Path, lib_path: &Path) -> Vec<String> {
    let output = Command::new(nm_path)
        .arg("--defined-only")
        .arg("--extern-only")
        .arg("--format=just-symbols")
        .arg(lib_path)
        .output()
        .expect("failed to run llvm-nm");
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("llvm-nm failed: {stderr}");
    }

    let stdout = String::from_utf8(output.stdout).expect("llvm-nm output is not valid UTF-8");
    let mut symbols: Vec<String> = stdout
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|s| !s.is_empty() && is_c_identifier(s))
        .collect();
    symbols.sort();
    symbols.dedup();
    symbols
}

/// C 識別子として有効かどうかを判定する
///
/// llvm-nm の --format=just-symbols 出力にはオブジェクトファイル名 (vp8_cx_iface.c.o: 等) も
/// 含まれるため、この関数で C 識別子のみをフィルタリングする。
///
/// macOS の Mach-O ではシンボル先頭に `_` が付くため、`_` で始まる文字列も受け入れる。
fn is_c_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// objcopy 用と bindgen 用のリネームマップを生成する
///
/// 2 つのマップを生成する理由:
///
/// objcopy_map: ライブラリバイナリ内の実シンボル名を書き換えるためのマップ。
///   macOS では _vpx_codec_encode → _shiguredo_vpx_codec_encode のようにプラットフォーム固有の
///   `_` プレフィックスを含む形で管理する。
///
/// bindgen_map: Rust バインディングの #[link_name] に使うマップ。
///   キーは C シンボル名 (vpx_codec_encode)、値はプラットフォーム固有のシンボル名
///   (_shiguredo_vpx_codec_encode) を格納する。
///   bindgen は generated_link_name_override の戻り値に \u{1} を付加してマングリングを
///   抑制するため、プラットフォーム固有の名前を直接返す必要がある。
fn build_symbol_rename_maps(
    symbols: &[String],
    is_macos: bool,
    rename_symbol: &dyn Fn(&str) -> Option<String>,
) -> SymbolRenameMaps {
    let mut objcopy_map = HashMap::new();
    let mut bindgen_map = HashMap::new();

    for sym in symbols {
        // プラットフォーム固有のプレフィックスを除去して C シンボル名を取得する
        //   macOS: _vpx_codec_encode → vpx_codec_encode
        //   Linux/Windows: vpx_codec_encode → vpx_codec_encode (変化なし)
        let c_name = if is_macos {
            sym.strip_prefix('_').unwrap_or(sym)
        } else {
            sym.as_str()
        };

        if let Some(new_c_name) = rename_symbol(c_name) {
            // objcopy 用: プラットフォーム固有のプレフィックスを再付与する
            //   macOS: shiguredo_vpx_codec_encode → _shiguredo_vpx_codec_encode
            //   Linux/Windows: shiguredo_vpx_codec_encode → shiguredo_vpx_codec_encode (変化なし)
            let new_sym = if is_macos {
                format!("_{new_c_name}")
            } else {
                new_c_name.clone()
            };
            objcopy_map.insert(sym.clone(), new_sym.clone());

            // bindgen 用: generated_link_name_override は \u{1} プレフィックスを付加して
            // シンボル名をそのまま使うため、プラットフォーム固有のシンボル名で管理する
            bindgen_map.insert(c_name.to_string(), new_sym);
        }
    }

    SymbolRenameMaps {
        objcopy_map,
        bindgen_map,
    }
}

/// --redefine-syms 用のマップファイルを書き出す
///
/// ファイル形式は 1 行に "旧シンボル名 新シンボル名" を空白区切りで記述する。
/// llvm-objcopy の --redefine-syms オプションで使用される。
fn write_objcopy_rename_map(map: &HashMap<String, String>, path: &Path) {
    let mut lines: Vec<String> = map
        .iter()
        .map(|(old, new)| format!("{old} {new}"))
        .collect();
    // 出力を決定的にするためソートする
    lines.sort();
    fs::write(path, lines.join("\n")).expect("failed to write symbol rename map");
}

/// llvm-objcopy でアーカイブ内のシンボルを書き換える
///
/// --redefine-syms はマップファイルに従ってシンボル名を一括置換する。
/// ライブラリファイルはインプレースで更新される。
fn rewrite_archive_symbols(objcopy_path: &Path, lib_path: &Path, map_file: &Path) {
    let status = Command::new(objcopy_path)
        .arg("--redefine-syms")
        .arg(map_file)
        .arg(lib_path)
        .status()
        .expect("failed to run llvm-objcopy");
    if !status.success() {
        panic!("llvm-objcopy failed");
    }
}

// --- 既存のヘルパー関数 ---

// CARGO_CFG_TARGET_OS + CARGO_CFG_TARGET_ARCH からプラットフォーム名を生成する
fn get_target_platform() -> String {
    if let Ok(target) = env::var("LIBVPX_TARGET") {
        return target;
    }

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    match (target_os.as_str(), target_arch.as_str()) {
        ("linux", "x86_64") => format!("{}_x86_64", detect_linux_distro()),
        ("linux", "aarch64") => format!("{}_arm64", detect_linux_distro()),
        ("macos", "aarch64") => "macos_arm64".to_string(),
        ("windows", "x86_64") if target_env == "gnu" => "windows_x86_64".to_string(),
        ("windows", "x86_64") => panic!(
            "the upstream windows_x86_64 prebuilt uses the GNU ABI; \
             enable shiguredo_libvpx/source-build for Windows MSVC"
        ),
        _ => panic!("unsupported target: os={}, arch={}", target_os, target_arch),
    }
}

// /etc/os-release から Ubuntu バージョンを検出する
fn detect_linux_distro() -> String {
    if let Ok(content) = fs::read_to_string("/etc/os-release") {
        for line in content.lines() {
            if let Some(version) = line.strip_prefix("VERSION_ID=") {
                let version = version.trim_matches('"');
                match version {
                    "22.04" | "24.04" | "26.04" => return format!("ubuntu-{}", version),
                    _ => {}
                }
            }
        }
    }
    panic!(
        "unsupported Linux distribution. \
         set LIBVPX_TARGET environment variable to specify the target explicitly"
    );
}

fn download_source_archive(build_dir: &Path) {
    let archive_path = build_dir.join("libvpx-source.tar.gz");
    let status = Command::new("curl")
        .args(["-fsSL", "-o"])
        .arg(&archive_path)
        .arg(SOURCE_ARCHIVE_URL)
        .status()
        .expect("failed to execute curl. Ensure curl is installed");
    if !status.success() {
        panic!("failed to download libvpx source archive: {SOURCE_ARCHIVE_URL}");
    }
    verify_sha256(&archive_path, SOURCE_ARCHIVE_SHA256);

    let status = Command::new("tar")
        .args(["xzf"])
        .arg(&archive_path)
        .arg("-C")
        .arg(build_dir)
        .status()
        .expect("failed to execute tar. Ensure tar is installed");
    if !status.success() {
        panic!("failed to extract libvpx source archive");
    }
    fs::rename(
        build_dir.join(SOURCE_ARCHIVE_DIRECTORY),
        build_dir.join(LIB_NAME),
    )
    .expect("failed to normalize libvpx source directory");
}

// Cargo.toml から依存ライブラリの URL とバージョンタグを取得する
fn get_url_and_version() -> (String, String) {
    let cargo_toml = shiguredo_toml::Value::Table(
        shiguredo_toml::from_str(include_str!("Cargo.toml")).expect("failed to parse Cargo.toml"),
    );
    if let Some((Some(url), Some(version))) = cargo_toml
        .get("package")
        .and_then(|v| v.get("metadata"))
        .and_then(|v| v.get("external-dependencies"))
        .and_then(|v| v.get(LIB_NAME))
        .map(|v| {
            (
                v.get("url").and_then(|s| s.as_str()),
                v.get("version").and_then(|s| s.as_str()),
            )
        })
    {
        (url.to_string(), version.to_string())
    } else {
        panic!(
            "Cargo.toml does not contain a valid [package.metadata.external-dependencies.{LIB_NAME}] table"
        );
    }
}
