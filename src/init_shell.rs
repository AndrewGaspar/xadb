use clap::IntoApp;
use quick_error::quick_error;

quick_error! {
    #[derive(Debug)]
    pub enum Error {
        ShellNotSupported
        Io(err: std::io::Error) {
            from()
        }
    }
}

fn bash_shell() -> Result<(), Error> {
    let mut cli = crate::cli::Args::command();

    let script = format!(
        r#"
xadb () {{
    export XADB_INIT_SHELL=bash
    export XADB_TEMP_FILE=$(mktemp /tmp/xadb-script.XXXXXX)
    {} $@
    source "${{XADB_TEMP_FILE}}"
    rm "${{XADB_TEMP_FILE}}"
    unset XADB_TEMP_FILE
    unset XADB_INIT_SHELL
}}
    "#,
        std::env::current_exe()?.to_str().unwrap()
    );

    println!("{script}");
    clap_complete::generate(
        clap_complete::Shell::Bash,
        &mut cli,
        "xadb",
        &mut std::io::stdout(),
    );

    Ok(())
}

fn zsh_shell() -> Result<(), Error> {
    let mut cli = crate::cli::Args::command();

    let script = format!(
        r#"
xadb () {{
    export XADB_INIT_SHELL=zsh
    export XADB_TEMP_FILE=$(mktemp /tmp/xadb-script.XXXXXX)
    {} $@
    source "${{XADB_TEMP_FILE}}"
    rm "${{XADB_TEMP_FILE}}"
    unset XADB_TEMP_FILE
    unset XADB_INIT_SHELL
}}
    "#,
        std::env::current_exe()?.to_str().unwrap()
    );

    println!("{script}");

    // this doesn't seem to work on mac :(
    clap_complete::generate_to(
        clap_complete::Shell::Zsh,
        &mut cli,
        "xadb",
        "/usr/local/share/zsh/site-functions",
    )?;

    Ok(())
}

pub fn init_shell(shell: &str) -> Result<(), Error> {
    match shell {
        "bash" => bash_shell(),
        "zsh" => zsh_shell(),
        _ => Err(Error::ShellNotSupported),
    }
}
