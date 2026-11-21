pub mod canvas;
pub mod dialogs;
pub mod menu;
pub mod win;

use std::path::{Path, PathBuf};

/// Returns the consolidated settings INI path (`winrussh.ini`) in the executable's directory.
pub fn get_config_path() -> PathBuf {
    if let Ok(mut exe_dir) = std::env::current_exe() {
        exe_dir.pop();
        return exe_dir.join("winrussh.ini");
    }
    PathBuf::from("winrussh.ini")
}

/// Inserts or replaces `[section]` in `existing` INI text without disturbing other sections.
pub fn upsert_ini_section(existing: &str, section: &str, new_section_text: &str) -> String {
    let target_header = format!("[{}]", section);
    let new_block = new_section_text.trim_end();
    let mut out_lines: Vec<String> = Vec::new();
    let mut in_target = false;
    let mut replaced = false;

    for line in existing.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if trimmed.eq_ignore_ascii_case(&target_header) {
                in_target = true;
                if !replaced {
                    for new_line in new_block.lines() {
                        out_lines.push(new_line.to_string());
                    }
                    replaced = true;
                }
            } else {
                in_target = false;
                out_lines.push(line.to_string());
            }
        } else if !in_target {
            out_lines.push(line.to_string());
        }
    }

    if !replaced {
        if let Some(last) = out_lines.last() {
            if !last.trim().is_empty() {
                out_lines.push(String::new());
            }
        }
        for new_line in new_block.lines() {
            out_lines.push(new_line.to_string());
        }
    }

    let mut result = out_lines.join("\r\n");
    if !result.is_empty() {
        result.push_str("\r\n");
    }
    result
}

pub fn save_ini_section(path: &Path, section: &str, new_section_text: &str) {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let updated = upsert_ini_section(&existing, section, new_section_text);
    let _ = std::fs::write(path, updated);
}

