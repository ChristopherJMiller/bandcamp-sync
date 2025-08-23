use clap::Command;
use clap_complete::{generate, Shell};
use std::io;

pub fn generate_completions(shell: Shell, cmd: &mut Command) {
    generate(shell, cmd, cmd.get_name().to_string(), &mut io::stdout());
}