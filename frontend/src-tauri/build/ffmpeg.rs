// ============================================================================
// FFmpeg Binary Bundling
// ============================================================================
// Download and bundle FFmpeg binaries at build-time to eliminate runtime download delays

/// Download and bundle FFmpeg binary for current target platform
/// Checks cache first, downloads only if missing or corrupted
pub fn ensure_ffmpeg_binary() {
    let target = std::env::var("TARGET")
        .or_else(|_| std::env::var("HOST"))
        .expect("Neither TARGET nor HOST environment variable set");

    println!(
        "cargo:warning=🎬 Checking FFmpeg binary for target: {}",
        target
    );

    let binary_name = if target.contains("windows") {
        format!("ffmpeg-{}.exe", target)
    } else {
        format!("ffmpeg-{}", target)
    };

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR environment variable not set");
    let binaries_dir = std::path::PathBuf::from(&manifest_dir).join("binaries");
    let binary_path = binaries_dir.join(&binary_name);

    // Cache check: Skip download if binary exists and works
    if binary_path.exists() {
        println!(
            "cargo:warning=🔍 Found cached FFmpeg binary: {}",
            binary_name
        );
        if verify_ffmpeg_binary(&binary_path) {
            println!(
                "cargo:warning=✅ FFmpeg binary already cached and verified: {}",
                binary_name
            );
            return;
        } else {
            println!("cargo:warning=⚠️  Cached FFmpeg binary appears corrupted, re-downloading...");
            let _ = std::fs::remove_file(&binary_path);
        }
    }

    println!(
        "cargo:warning=📥 FFmpeg binary not found, downloading for {}",
        target
    );

    // Create binaries directory if it doesn't exist
    if !binaries_dir.exists() {
        std::fs::create_dir_all(&binaries_dir).expect("Failed to create binaries directory");
    }

    // Download and extract
    match download_and_extract_ffmpeg(&target, &binary_path) {
        Ok(()) => {
            println!(
                "cargo:warning=✅ FFmpeg binary downloaded successfully: {}",
                binary_name
            );

            // Verify downloaded binary works
            if !verify_ffmpeg_binary(&binary_path) {
                panic!("⚠️  Downloaded FFmpeg binary verification failed!");
            }
        }
        Err(e) => {
            panic!("⚠️  Failed to download FFmpeg: {}", e);
        }
    }
}

/// Download FFmpeg from platform-specific URL and extract to target location
fn download_and_extract_ffmpeg(
    target: &str,
    output_path: &std::path::PathBuf,
) -> Result<(), String> {
    use std::io::Write;

    println!(
        "cargo:warning=🌐 Fetching FFmpeg download URL for {}",
        target
    );

    // Get platform-specific download URL
    let url = get_ffmpeg_url_for_target(target)?;

    println!("cargo:warning=⬇️  Downloading from: {}", url);

    // Download with timeout (using reqwest from build-dependencies)
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(600)) // 10 min timeout for large downloads
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(&url)
        .send()
        .map_err(|e| format!("Failed to download: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("HTTP error: {}", response.status()));
    }

    let total_size = response.content_length().unwrap_or(0);
    println!(
        "cargo:warning=📦 Download size: {:.1} MB",
        total_size as f64 / 1_048_576.0
    );

    // Download to temp file
    let temp_dir = std::env::temp_dir();
    let archive_filename = url.split('/').next_back().unwrap_or("ffmpeg-archive");
    let archive_path = temp_dir.join(format!("ffmpeg-build-{}-{}", target, archive_filename));

    {
        let content = response
            .bytes()
            .map_err(|e| format!("Failed to read response: {}", e))?;

        // Integrity check (specs/0028): verify the SHA-256 of the downloaded
        // archive BEFORE we trust or extract it. When a digest is pinned this
        // fails the build on mismatch (tamper/CDN-swap protection); until one is
        // pinned it downgrades to a loud "unverified" warning that prints the
        // actual digest so it can be filled in. See verify_archive_sha256.
        verify_archive_sha256(&content, target, &url)?;

        let mut file = std::fs::File::create(&archive_path)
            .map_err(|e| format!("Failed to create temp file: {}", e))?;

        file.write_all(&content)
            .map_err(|e| format!("Failed to write archive: {}", e))?;
    }

    println!("cargo:warning=📦 Downloaded to: {:?}", archive_path);
    println!("cargo:warning=📂 Extracting FFmpeg binary...");

    // Extract binary (platform-specific)
    extract_ffmpeg_from_archive(&archive_path, target, output_path)?;

    // Cleanup archive
    let _ = std::fs::remove_file(&archive_path);

    println!("cargo:warning=✨ Extraction complete");

    Ok(())
}

/// Get FFmpeg download URL for specific target triple
fn get_ffmpeg_url_for_target(target: &str) -> Result<String, String> {
    // Platform-specific URLs
    let url = if target.contains("windows") {
        // Windows
        "https://github.com/Zackriya-Solutions/ffmpeg-binaries/releases/download/0.0.1/ffmpeg-8.0.1-essentials_build.zip"
    } else if target.contains("apple") {
        if target.contains("aarch64") {
            // Apple Silicon (M1/M2/M3)
            "https://github.com/Zackriya-Solutions/ffmpeg-binaries/releases/download/0.0.1/ffmpeg80arm.zip"
        } else {
            // Intel Mac
            "https://github.com/Zackriya-Solutions/ffmpeg-binaries/releases/download/0.0.1/ffmpeg-8.0.1.zip"
        }
    } else if target.contains("linux") {
        if target.contains("aarch64") || target.contains("arm") {
            // Linux ARM64
            "https://github.com/Zackriya-Solutions/ffmpeg-binaries/releases/download/0.0.1/ffmpeg-release-arm64-static.tar.xz"
        } else {
            // Linux x86_64
            "https://github.com/Zackriya-Solutions/ffmpeg-binaries/releases/download/0.0.1/ffmpeg-release-amd64-static.tar.xz"
        }
    } else {
        return Err(format!("Unsupported target platform: {}", target));
    };

    Ok(url.to_string())
}

/// Extract FFmpeg binary from downloaded archive (handles ZIP and TAR.XZ)
fn extract_ffmpeg_from_archive(
    archive_path: &std::path::Path,
    target: &str,
    output_path: &std::path::PathBuf,
) -> Result<(), String> {
    let extract_dir = std::env::temp_dir().join(format!("ffmpeg-extract-{}", target));

    // Clean old extraction directory
    let _ = std::fs::remove_dir_all(&extract_dir);
    std::fs::create_dir_all(&extract_dir)
        .map_err(|e| format!("Failed to create extract dir: {}", e))?;

    // Determine archive format from extension
    let archive_str = archive_path.to_string_lossy();

    if archive_str.ends_with(".zip") {
        extract_zip(archive_path, &extract_dir)?;
    } else if archive_str.ends_with(".tar.xz") || archive_str.ends_with(".txz") {
        extract_tar_xz(archive_path, &extract_dir)?;
    } else {
        return Err(format!("Unsupported archive format: {}", archive_str));
    }

    // Find extracted FFmpeg binary (platform-specific locations)
    let ffmpeg_binary = find_ffmpeg_in_extracted_dir(&extract_dir, target)?;

    println!("cargo:warning=📋 Found FFmpeg at: {:?}", ffmpeg_binary);

    // Copy to target location
    std::fs::copy(&ffmpeg_binary, output_path)
        .map_err(|e| format!("Failed to copy binary to binaries/: {}", e))?;

    // Set executable permissions on Unix systems
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(output_path)
            .map_err(|e| format!("Failed to get metadata: {}", e))?
            .permissions();
        perms.set_mode(0o755); // rwxr-xr-x
        std::fs::set_permissions(output_path, perms)
            .map_err(|e| format!("Failed to set executable permissions: {}", e))?;
        println!("cargo:warning=🔐 Set executable permissions");
    }

    // Cleanup extraction directory
    let _ = std::fs::remove_dir_all(&extract_dir);

    Ok(())
}

/// Extract ZIP archive (Windows, macOS)
fn extract_zip(
    archive_path: &std::path::Path,
    extract_dir: &std::path::Path,
) -> Result<(), String> {
    let file =
        std::fs::File::open(archive_path).map_err(|e| format!("Failed to open ZIP: {}", e))?;

    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("Failed to read ZIP archive: {}", e))?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read ZIP entry {}: {}", i, e))?;

        // Use enclosed_name() to prevent Zip Slip path traversal attacks
        let outpath = match file.enclosed_name() {
            Some(name) => extract_dir.join(name),
            None => {
                // Skip entries with path traversal sequences (e.g., "../")
                println!(
                    "cargo:warning=⚠️  Skipping suspicious ZIP entry: {}",
                    file.name()
                );
                continue;
            }
        };

        if file.is_dir() {
            // Directory
            std::fs::create_dir_all(&outpath)
                .map_err(|e| format!("Failed to create directory: {}", e))?;
        } else {
            // File
            if let Some(parent) = outpath.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create parent directory: {}", e))?;
            }

            let mut outfile = std::fs::File::create(&outpath)
                .map_err(|e| format!("Failed to create output file: {}", e))?;

            std::io::copy(&mut file, &mut outfile)
                .map_err(|e| format!("Failed to extract file: {}", e))?;
        }

        // Set Unix permissions if available
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = file.unix_mode() {
                std::fs::set_permissions(&outpath, std::fs::Permissions::from_mode(mode)).ok();
            }
        }
    }

    Ok(())
}

/// Extract TAR.XZ archive (Linux)
fn extract_tar_xz(
    archive_path: &std::path::Path,
    extract_dir: &std::path::Path,
) -> Result<(), String> {
    let file =
        std::fs::File::open(archive_path).map_err(|e| format!("Failed to open TAR.XZ: {}", e))?;

    // Decompress XZ
    let decompressor = xz2::read::XzDecoder::new(file);

    // Extract TAR
    let mut archive = tar::Archive::new(decompressor);
    archive
        .unpack(extract_dir)
        .map_err(|e| format!("Failed to extract TAR: {}", e))?;

    Ok(())
}

/// Find FFmpeg binary in extracted directory (handles nested structures)
fn find_ffmpeg_in_extracted_dir(
    extract_dir: &std::path::Path,
    target: &str,
) -> Result<std::path::PathBuf, String> {
    let executable_name = if target.contains("windows") {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };

    // Search patterns (in priority order)
    let search_patterns = [
        extract_dir.join(executable_name),             // Flat: ffmpeg
        extract_dir.join("bin").join(executable_name), // Nested: bin/ffmpeg
    ];

    // Try direct paths first
    for pattern in &search_patterns {
        if pattern.exists() && pattern.is_file() {
            return Ok(pattern.clone());
        }
    }

    // Recursive search for nested directories (e.g., ffmpeg-6.0-full_build/bin/ffmpeg.exe)
    for entry in
        std::fs::read_dir(extract_dir).map_err(|e| format!("Failed to read extract dir: {}", e))?
    {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let path = entry.path();

        if path.is_dir() {
            // Check bin/ subdirectory
            let bin_path = path.join("bin").join(executable_name);
            if bin_path.exists() && bin_path.is_file() {
                return Ok(bin_path);
            }

            // Check root of subdirectory
            let root_path = path.join(executable_name);
            if root_path.exists() && root_path.is_file() {
                return Ok(root_path);
            }
        }
    }

    Err(format!(
        "FFmpeg binary '{}' not found in extracted archive",
        executable_name
    ))
}

// ============================================================================
// Download integrity verification (specs/0028)
// ============================================================================

/// Expected SHA-256 (lowercase hex) of the downloaded FFmpeg archive, keyed by
/// target triple. These correspond to the exact artifacts in
/// `get_ffmpeg_url_for_target` (Zackriya-Solutions/ffmpeg-binaries release tag
/// `0.0.1` — all five URLs are pinned to that immutable tag path, so a digest
/// here pins the exact bytes even if the tag's assets were ever replaced).
///
/// Each digest was computed 2026-07-01 from two independent downloads of the
/// exact URL (both fetches matched):
///     curl -sSL <url> | shasum -a 256
/// Returns `None` only for target triples we don't ship FFmpeg for (those are
/// rejected earlier by `get_ffmpeg_url_for_target`).
fn expected_ffmpeg_sha256(target: &str) -> Option<&'static str> {
    if target.contains("windows") {
        // ffmpeg-8.0.1-essentials_build.zip
        Some("e2aaeaa0fdbc397d4794828086424d4aaa2102cef1fb6874f6ffd29c0b88b673")
    } else if target.contains("apple") {
        if target.contains("aarch64") {
            // ffmpeg80arm.zip (Apple Silicon)
            Some("0d4efcaf6a098430a708e0af694a84792938921fa126162787ae98c6151d7a95")
        } else {
            // ffmpeg-8.0.1.zip (Intel Mac)
            Some("470e482f6e290eac92984ac12b2d67bad425b1e5269fd75fb6a3536c16e824e4")
        }
    } else if target.contains("linux") {
        if target.contains("aarch64") || target.contains("arm") {
            // ffmpeg-release-arm64-static.tar.xz (release tag 0.0.1 asset)
            Some("f4149bb2b0784e30e99bdda85471c9b5930d3402014e934a5098b41d0f7201b1")
        } else {
            // ffmpeg-release-amd64-static.tar.xz (release tag 0.0.1 asset)
            Some("abda8d77ce8309141f83ab8edf0596834087c52467f6badf376a6a2a4c87cf67")
        }
    } else {
        None
    }
}

/// Verify the downloaded archive against its pinned SHA-256. This is a hard
/// gate: a mismatch — or a supported target with no pinned digest — fails the
/// build before the archive is trusted or extracted. This is the pre-extraction
/// supply-chain check; the post-copy `-version` check remains as a functional
/// sanity check.
fn verify_archive_sha256(bytes: &[u8], target: &str, url: &str) -> Result<(), String> {
    let actual = sha256_hex(bytes);

    match expected_ffmpeg_sha256(target) {
        Some(expected) => {
            let expected = expected.trim().to_lowercase();
            if actual == expected {
                println!(
                    "cargo:warning=🔒 FFmpeg archive SHA-256 verified for {}",
                    target
                );
                Ok(())
            } else {
                Err(format!(
                    "FFmpeg archive SHA-256 mismatch for {target}\n  url:      {url}\n  \
                     expected: {expected}\n  actual:   {actual}\n\
                     Refusing to use a tampered/unexpected download. If the upstream \
                     release asset was legitimately updated, re-verify it out-of-band \
                     and update expected_ffmpeg_sha256() in build/ffmpeg.rs."
                ))
            }
        }
        None => Err(format!(
            "No pinned SHA-256 for FFmpeg target {target} (url: {url}, actual \
             SHA-256 of download: {actual}). Refusing an unverified download: \
             verify the archive out-of-band and add its digest to \
             expected_ffmpeg_sha256() in build/ffmpeg.rs."
        )),
    }
}

/// Dependency-free SHA-256 (FIPS 180-4). Implemented inline because the crate's
/// `[build-dependencies]` are owned elsewhere this run (specs/0028) and cannot
/// take a new `sha2` dep; a build script running on macOS/Linux/Windows also
/// can't rely on a system hashing tool. Returns lowercase hex.
fn sha256_hex(data: &[u8]) -> String {
    #[rustfmt::skip]
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];

    #[rustfmt::skip]
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];

    // Pre-processing: append 0x80, pad with zeros to 56 mod 64, then 64-bit length.
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in w.iter_mut().enumerate().take(16) {
            let b = i * 4;
            *word = u32::from_be_bytes([chunk[b], chunk[b + 1], chunk[b + 2], chunk[b + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = String::with_capacity(64);
    for word in &h {
        out.push_str(&format!("{word:08x}"));
    }
    out
}

/// Verify FFmpeg binary is functional (runs -version successfully)
fn verify_ffmpeg_binary(path: &std::path::PathBuf) -> bool {
    match std::process::Command::new(path).arg("-version").output() {
        Ok(output) => {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if let Some(version_line) = stdout.lines().next() {
                    println!(
                        "cargo:warning=✅ FFmpeg verification passed: {}",
                        version_line
                    );
                }
                true
            } else {
                false
            }
        }
        Err(_) => false,
    }
}
