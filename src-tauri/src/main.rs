// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(windows)]
    {
        let mut arguments = std::env::args_os().skip(1);
        let command = arguments.next();
        if command.as_deref() == Some(std::ffi::OsStr::new("--uninstall-data")) {
            let chinese = arguments.next().as_deref() == Some(std::ffi::OsStr::new("2052"));
            if let Err(error) = qookix_launcher_lib::uninstall_data::run(chinese) {
                eprintln!("{error}");
                std::process::exit(4);
            }
            return;
        }
        if command.as_deref() == Some(std::ffi::OsStr::new("--commit-install-data")) {
            let result = match (arguments.next(), arguments.next(), arguments.next()) {
                (Some(data), Some(install), None) => qookix_launcher_lib::commit_install_data(
                    std::path::Path::new(&data), std::path::Path::new(&install),
                ),
                _ => Err("安装器缺少目录参数".into()),
            };
            if let Err(error) = result {
                eprintln!("{error}");
                std::process::exit(3);
            }
            return;
        }
    }
    qookix_launcher_lib::run()
}
