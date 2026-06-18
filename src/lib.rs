use std::path::PathBuf;

use zed_extension_api::{
    self as zed, serde_json, Architecture, GithubReleaseOptions, LanguageServerId, Os, Worktree,
};

struct CodeTrackrExtension {
    cached_binary_path: Option<String>,
}

impl CodeTrackrExtension {
    fn download_binary(&self, worktree: &Worktree) -> zed::Result<String> {
        // La extension se inicializa con PWD como working directory.
        // download_file solo puede escribir dentro de ese directorio,
        // asi que usamos rutas relativas y construimos la ruta absoluta.
        let cwd = std::env::current_dir()
            .map_err(|e| format!("Error obteniendo directorio de trabajo: {e}"))?;
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

        // 3. Obtener la release desde GitHub
        let release = zed::latest_github_release(
            "livrasand/codetrackr-zed",
            GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )
        .map_err(|e| {
            format!(
                "No hay releases disponibles en GitHub: {e}\n\n\
                 Para usar CodeTrackr, compila el language server manualmente:\n\
                 cd ls && cargo build --release\n\
                 y copia el binario a un directorio en tu PATH"
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
                     Compilacion manual:\n\
                     cd ls && cargo build --release"
                )
            })?;

        // 4. Descargar usando nombre relativo (download_file solo permite
        //    escribir dentro del working directory de la extension)
        let binary_name = match os {
            Os::Windows => "codetrackr-ls.exe",
            _ => "codetrackr-ls",
        };

        zed::download_file(
            &asset.download_url,
            binary_name,
            zed::DownloadedFileType::Uncompressed,
        )
        .map_err(|e| {
            format!(
                "Error descargando codetrackr-ls ({target}): {e}\n\n\
                 Compilacion manual:\n\
                 cd ls && cargo build --release"
            )
        })?;

        zed::make_file_executable(binary_name)
            .map_err(|e| format!("Error haciendo ejecutable codetrackr-ls: {e}"))?;

        let bin_path = cwd.join(binary_name);
        let bin_path_str = bin_path
            .to_str()
            .ok_or_else(|| "Ruta invalida".to_string())?
            .to_string();

        Ok(bin_path_str)
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
