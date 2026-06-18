use zed_extension_api::{
    self as zed, serde_json, Architecture, GithubReleaseOptions, LanguageServerId, Os, Worktree,
};

struct CodeTrackrExtension {
    cached_binary_path: Option<String>,
}

impl CodeTrackrExtension {
    fn download_binary(&self, worktree: &Worktree) -> zed::Result<String> {
        // 1. Si ya esta en el PATH del usuario, usarlo directamente
        if let Some(path) = worktree.which("codetrackr-ls") {
            return Ok(path);
        }

        // 2. Detectar plataforma actual
        let (os, arch) = zed::current_platform();
        let target = format!(
            "{}-{}",
            match arch {
                Architecture::Aarch64 => "aarch64",
                Architecture::X8664 => "x86_64",
                Architecture::X86 => "x86",
            },
            match os {
                Os::Mac => "apple-darwin",
                Os::Linux => "unknown-linux-gnu",
                Os::Windows => "pc-windows-msvc",
            }
        );

        let asset_name = format!("codetrackr-ls-{target}");

        // 3. Obtener la ultima release desde GitHub (incluye prereleases)
        let release = zed::latest_github_release(
            "livrasand/codetrackr-zed",
            GithubReleaseOptions {
                require_assets: true,
                pre_release: true,
            },
        )
        .map_err(|e| {
            format!(
                "No hay releases disponibles en GitHub: {e}\n\n\
                 Para usar CodeTrackr, compila el language server manualmente:\n\
                 cd ls && cargo build --release && \
                 cp target/release/codetrackr-ls /usr/local/bin/\n\
                 O crea un release en https://github.com/livrasand/codetrackr-zed/releases"
            )
        })?;

        let asset = release
            .assets
            .iter()
            .find(|a| a.name == asset_name)
            .ok_or_else(|| {
                format!(
                    "No hay binario pre-compilado para {target}. \
                     Asset esperado: {asset_name}\n\n\
                     Si aun no hay releases, compila manualmente:\n\
                     cd ls && cargo build --release && \
                     cp target/release/codetrackr-ls /usr/local/bin/"
                )
            })?;

        // 4. Descargar a un lugar persistente
        let env = worktree.shell_env();
        let home = get_env(&env, "HOME")
            .ok_or_else(|| "Variable HOME no encontrada en el entorno".to_string())?;

        let bin_path = format!("{home}/.local/share/codetrackr/bin/codetrackr-ls");

        zed::download_file(
            &asset.download_url,
            &bin_path,
            zed::DownloadedFileType::Uncompressed,
        )
        .map_err(|e| {
            format!(
                "Error descargando codetrackr-ls ({target}): {e}\n\n\
                 Instalacion manual:\n\
                 cd ls && cargo build --release && \
                 cp target/release/codetrackr-ls /usr/local/bin/"
            )
        })?;

        zed::make_file_executable(&bin_path)
            .map_err(|e| format!("Error haciendo ejecutable codetrackr-ls: {e}"))?;

        Ok(bin_path)
    }
}

impl zed::Extension for CodeTrackrExtension {
    fn new() -> Self {
        CodeTrackrExtension {
            cached_binary_path: None,
        }
    }

    fn language_server_command(
        &mut self,
        _language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<zed::Command> {
        let path = if let Some(cached) = &self.cached_binary_path {
            cached.clone()
        } else {
            let path = self.download_binary(worktree)?;
            self.cached_binary_path = Some(path.clone());
            path
        };

        let env = worktree.shell_env();
        let api_key = get_env(&env, "CODETRACKR_API_KEY").unwrap_or_default();
        let base_url = get_env(&env, "CODETRACKR_BASE_URL")
            .unwrap_or_else(|| "https://codetrackr.fly.dev".to_string());

        Ok(zed::Command {
            command: path,
            args: vec![],
            env: vec![
                ("CODETRACKR_API_KEY".to_string(), api_key),
                ("CODETRACKR_BASE_URL".to_string(), base_url),
            ],
        })
    }

    fn language_server_workspace_configuration(
        &mut self,
        _language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> zed::Result<Option<serde_json::Value>> {
        let env = worktree.shell_env();
        let api_key = get_env(&env, "CODETRACKR_API_KEY").unwrap_or_default();
        let base_url = get_env(&env, "CODETRACKR_BASE_URL")
            .unwrap_or_else(|| "https://codetrackr.fly.dev".to_string());

        Ok(Some(serde_json::json!({
            "api_key": api_key,
            "base_url": base_url,
        })))
    }
}

fn get_env(env: &[(String, String)], key: &str) -> Option<String> {
    env.iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
        .filter(|v| !v.is_empty())
}

zed::register_extension!(CodeTrackrExtension);
