use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Debug)]
pub struct CmdOutput {
    pub code: i32,
    pub text: String,
    pub stderr: String,
}

pub fn run(program: &Path, args: &[String]) -> std::io::Result<CmdOutput> {
    let output = Command::new(program)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .output()?;
    Ok(CmdOutput {
        code: output.status.code().unwrap_or(-1),
        text: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

pub fn run_named(program: &str, args: &[String]) -> std::io::Result<CmdOutput> {
    run(Path::new(program), args)
}

/// PowerShell 单引号字面量：内部单引号需要写成两个。
pub fn ps_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}
