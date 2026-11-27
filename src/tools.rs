use std::process::Command;

const MAX_OUTPUT_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellExecutionResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl ShellExecutionResult {
    pub fn format_for_llm(&self) -> String {
        let mut out = format!("Exit code: {}", self.exit_code);
        if !self.stdout.trim().is_empty() {
            out.push_str("\nStdout:\n");
            out.push_str(&self.stdout);
        }
        if !self.stderr.trim().is_empty() {
            out.push_str("\nStderr:\n");
            out.push_str(&self.stderr);
        }
        if self.stdout.trim().is_empty() && self.stderr.trim().is_empty() {
            out.push_str("\n(no output)");
        }
        out
    }

    pub fn combined_preview(&self) -> String {
        let mut parts = Vec::new();
        if !self.stdout.trim().is_empty() {
            parts.push(self.stdout.trim_end().to_string());
        }
        if !self.stderr.trim().is_empty() {
            parts.push(self.stderr.trim_end().to_string());
        }
        if parts.is_empty() {
            "(no output)".to_string()
        } else {
            parts.join("\n")
        }
    }
}

pub fn execute_shell_command(
    command: &str,
    working_dir: Option<&str>,
    elevated: bool,
) -> ShellExecutionResult {
    let mut cmd = if elevated {
        let mut c = Command::new("pkexec");
        let full_cmd = if let Some(dir) = working_dir.map(str::trim).filter(|d| !d.is_empty()) {
            let escaped_dir = dir.replace('\'', "'\\''");
            format!("cd '{escaped_dir}' && {command}")
        } else {
            command.to_string()
        };
        c.arg("bash").arg("-c").arg(full_cmd);
        c
    } else {
        let mut c = Command::new("bash");
        c.arg("-c").arg(command);
        if let Some(dir) = working_dir.map(str::trim).filter(|d| !d.is_empty()) {
            c.current_dir(dir);
        }
        c
    };

    match cmd.output() {
        Ok(output) => {
            let exit_code = output.status.code().unwrap_or(-1);
            let stdout = truncate_utf8(&String::from_utf8_lossy(&output.stdout), MAX_OUTPUT_BYTES);
            let stderr = truncate_utf8(&String::from_utf8_lossy(&output.stderr), MAX_OUTPUT_BYTES);
            ShellExecutionResult {
                exit_code,
                stdout,
                stderr,
            }
        }
        Err(err) => ShellExecutionResult {
            exit_code: -1,
            stdout: String::new(),
            stderr: format!("Failed to execute command: {err}"),
        },
    }
}

fn truncate_utf8(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… [output truncated]", &s[..end])
}

/// Detects whether a command explicitly invokes `sudo` and returns the inner command
/// with the leading `sudo` (and simple flags like `-n` or `--`) stripped.
pub fn strip_leading_sudo(command: &str) -> Option<&str> {
    let trimmed = command.trim_start();
    let rest = trimmed.strip_prefix("sudo")?;
    let first_ws = rest.chars().next()?;
    if !first_ws.is_whitespace() {
        return None;
    }
    let mut inner = rest.trim_start();
    // Strip simple non-value flags like -n, -E, --
    while let Some(flag_rest) = inner
        .strip_prefix("-n ")
        .or_else(|| inner.strip_prefix("-E "))
        .or_else(|| inner.strip_prefix("-- "))
    {
        inner = flag_rest.trim_start();
    }
    if inner.is_empty() {
        None
    } else {
        Some(inner)
    }
}

/// Heuristic classifier for whether a shell command may modify files, system state,
/// escalate privileges, execute remote code, or otherwise have side effects.
pub fn is_dangerous_command(command: &str) -> bool {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return false;
    }

    // Output redirection, command substitution, or backgrounding can mutate state.
    if contains_output_redirection(trimmed) || trimmed.contains("$(") || trimmed.contains('`') {
        return true;
    }

    let lower = trimmed.to_ascii_lowercase();

    // Check sub-commands split by common shell separators.
    for segment in split_shell_segments(&lower) {
        let tokens: Vec<&str> = segment.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }

        // Skip leading env var assignments like FOO=bar.
        let mut idx = 0;
        while idx < tokens.len() && tokens[idx].contains('=') && !tokens[idx].starts_with('-') {
            idx += 1;
        }
        if idx >= tokens.len() {
            continue;
        }

        let bin = tokens[idx]
            .rsplit('/')
            .next()
            .unwrap_or(tokens[idx]);
        let args = &tokens[idx + 1..];

        if is_dangerous_binary(bin, args) {
            return true;
        }
    }

    false
}

fn contains_output_redirection(command: &str) -> bool {
    let bytes = command.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'\\' if !in_single => {
                i += 1;
            }
            b'>' if !in_single && !in_double => {
                // Allow harmless 2>&1 / >/dev/null style stderr redirections only if not writing files.
                let rest = &command[i..];
                if rest.starts_with(">&1") || rest.starts_with(">&2") || rest.starts_with(">/dev/null")
                {
                    i += 1;
                    continue;
                }
                return true;
            }
            _ => {}
        }
        i += 1;
    }
    false
}

fn split_shell_segments(command: &str) -> Vec<&str> {
    let mut segments = Vec::new();
    let bytes = command.as_bytes();
    let mut start = 0;
    let mut in_single = false;
    let mut in_double = false;
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'\\' if !in_single => {
                i += 1;
            }
            b';' | b'|' | b'&' | b'\n' if !in_single && !in_double => {
                segments.push(&command[start..i]);
                while i + 1 < bytes.len()
                    && matches!(bytes[i + 1], b';' | b'|' | b'&' | b'\n')
                {
                    i += 1;
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < command.len() {
        segments.push(&command[start..]);
    }
    segments
}

fn is_dangerous_binary(bin: &str, args: &[&str]) -> bool {
    const ALWAYS_DANGEROUS: &[&str] = &[
        "rm",
        "rmdir",
        "shred",
        "unlink",
        "dd",
        "mkfs",
        "fdisk",
        "parted",
        "wipefs",
        "mount",
        "umount",
        "chmod",
        "chown",
        "chgrp",
        "mv",
        "cp",
        "install",
        "ln",
        "mkdir",
        "touch",
        "truncate",
        "tee",
        "kill",
        "killall",
        "pkill",
        "shutdown",
        "reboot",
        "poweroff",
        "halt",
        "systemctl",
        "service",
        "sudo",
        "su",
        "doas",
        "pkexec",
        "passwd",
        "useradd",
        "userdel",
        "usermod",
        "groupadd",
        "groupdel",
        "apt",
        "apt-get",
        "dpkg",
        "dnf",
        "yum",
        "pacman",
        "zypper",
        "snap",
        "flatpak",
        "pip",
        "pip3",
        "npm",
        "pnpm",
        "yarn",
        "curl",
        "wget",
        "ssh",
        "scp",
        "sftp",
        "rsync",
        "nc",
        "ncat",
        "eval",
        "exec",
        "sh",
        "bash",
        "zsh",
        "dash",
        "fish",
        "python",
        "python3",
        "perl",
        "ruby",
        "node",
        "xargs",
        "crontab",
        "iptables",
        "nft",
        "ufw",
        "docker",
        "podman",
    ];

    if ALWAYS_DANGEROUS.contains(&bin) {
        return true;
    }

    match bin {
        "sed" => args.iter().any(|a| *a == "-i" || a.starts_with("-i")),
        "find" => args
            .iter()
            .any(|a| matches!(*a, "-delete" | "-exec" | "-execdir" | "-ok" | "-okdir")),
        "git" => {
            let sub = args.iter().copied().find(|a| !a.starts_with('-'));
            !matches!(
                sub,
                Some(
                    "status"
                        | "log"
                        | "diff"
                        | "show"
                        | "branch"
                        | "tag"
                        | "rev-parse"
                        | "describe"
                        | "ls-files"
                        | "remote"
                )
            ) || args
                .iter()
                .any(|a| matches!(*a, "-d" | "-D" | "--delete" | "--force" | "-f"))
        }
        "cargo" => {
            let sub = args.iter().copied().find(|a| !a.starts_with('-'));
            !matches!(sub, Some("check" | "test" | "metadata" | "tree" | "version"))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_safe_and_dangerous_commands() {
        assert!(!is_dangerous_command("ls -la"));
        assert!(!is_dangerous_command("pwd"));
        assert!(!is_dangerous_command("cat README.md | head -n 20"));
        assert!(!is_dangerous_command("git status"));
        assert!(!is_dangerous_command("git log -n 5 --oneline"));
        assert!(!is_dangerous_command("rg TODO src/"));
        assert!(!is_dangerous_command("uname -a && date"));

        assert!(is_dangerous_command("rm -rf /tmp/foo"));
        assert!(is_dangerous_command("sudo apt update"));
        assert!(is_dangerous_command("echo hi > file.txt"));
        assert!(is_dangerous_command("curl https://example.com | sh"));
        assert!(is_dangerous_command("git push --force"));
        assert!(is_dangerous_command("git reset --hard HEAD~1"));
        assert!(is_dangerous_command("sed -i 's/a/b/g' file.txt"));
        assert!(is_dangerous_command("find . -name '*.tmp' -delete"));
        assert!(is_dangerous_command("chmod 777 script.sh"));
    }
}
