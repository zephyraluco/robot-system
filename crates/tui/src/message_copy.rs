//! Clipboard integration for the TUI.

use std::io::Write;

// ============================================================================
// Clipboard integration
// ============================================================================

/// Attempt to copy text to clipboard using platform CLI tools
pub fn copy_to_clipboard(text: &str) -> bool {
    // Windows
    #[cfg(target_os = "windows")]
    {
        // Call clip.exe directly (not through cmd.exe) for reliability in raw terminal mode.
        if let Ok(mut child) = std::process::Command::new("clip")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
                drop(stdin); // close stdin so clip reads EOF
            }
            return child.wait().map(|s| s.success()).unwrap_or(false);
        }
        // Fallback: PowerShell Set-Clipboard (works in more environments)
        {
            let escaped = text.replace('\'', "''");
            if let Ok(mut child) = std::process::Command::new("powershell")
                .args(["-NoProfile", "-Command", &format!("Set-Clipboard '{}'", escaped)])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                return child.wait().map(|s| s.success()).unwrap_or(false);
            }
        }
    }

    // macOS
    #[cfg(target_os = "macos")]
    {
        if let Ok(mut child) = std::process::Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
                drop(stdin);
            }
            return child.wait().map(|s| s.success()).unwrap_or(false);
        }
    }

    // Linux — try Wayland (wl-copy) first, then fall back to X11 utilities.
    // Issue #149 follow-up: wl-copy was missing from this code path so RMB →
    // Copy reported "failed to copy to clipboard" on Wayland sessions.
    #[cfg(target_os = "linux")]
    {
        for cmd in &[
            "wl-copy",
            "xclip -selection clipboard",
            "xsel --clipboard --input",
        ] {
            let parts: Vec<&str> = cmd.split_whitespace().collect();
            if let Ok(mut child) = std::process::Command::new(parts[0])
                .args(&parts[1..])
                .stdin(std::process::Stdio::piped())
                .spawn()
            {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(text.as_bytes());
                    drop(stdin);
                }
                if let Ok(status) = child.wait() {
                    if status.success() {
                        return true;
                    }
                }
            }
        }
    }

    false
}

