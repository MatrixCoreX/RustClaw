use std::fs;
use std::path::{Path, PathBuf};

use rustclaw_skill_sdk::adapter::{copy_source_tree, source_digest};
use rustclaw_skill_sdk::{
    scaffold_skill, validate_response_line, AdoptBuiltRequest, ImplementationLanguage,
    InstallReceiptStore, InstallRequest, PackageManifest, ScaffoldRequest, SkillInstaller,
    SkillRuntimeResolver, SkillSdkError,
};
use serde_json::{json, Value};

fn main() {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    let human = take_flag(&mut args, "--human");
    let result = run(args);
    match result {
        Ok(value) => {
            if human {
                println!("{}", human_summary(&value));
            } else {
                println!(
                    "{}",
                    serde_json::to_string(&value).expect("encode CLI result")
                );
            }
        }
        Err(error) => {
            if human {
                eprintln!("{}: {}", error.code, error.detail);
            } else {
                println!(
                    "{}",
                    serde_json::to_string(&json!({"ok": false, "error": error}))
                        .expect("encode CLI error")
                );
            }
            std::process::exit(1);
        }
    }
}

fn run(args: Vec<String>) -> Result<Value, SkillSdkError> {
    let command = args.first().map(String::as_str).unwrap_or("help");
    match command {
        "init" => init_command(&args),
        "validate" => validate_command(&args),
        "build" | "protocol-test" | "install-local" => install_command(command, &args),
        "adopt-built" => adopt_built_command(&args),
        "package" => package_command(&args),
        "protocol-validate" => protocol_validate_command(&args),
        "receipt-verify" => receipt_verify_command(&args),
        "rollback" => rollback_command(&args),
        "help" | "--help" | "-h" => Ok(json!({
            "ok": true,
            "commands": [
                "init <rust|python|node|go|prebuilt|generic_process|http_json> <skill_name> <destination>",
                "validate <skill.toml>",
                "build <skill.toml> <workspace_root> <package_root> [--network] [--target <triple>]",
                "protocol-test <skill.toml> <workspace_root> <package_root> [--network] [--target <triple>]",
                "package <skill.toml> <output_dir>",
                "install-local <skill.toml> <workspace_root> <package_root> [--network] [--target <triple>]",
                "adopt-built <skill.toml> <workspace_root> <package_root> <binary_path> [--target <triple>]",
                "protocol-validate <request_id> <response.jsonl>",
                "receipt-verify <package_root> <skill_name>",
                "rollback <package_root> <skill_name>"
            ],
            "output": "JSON by default; add --human for concise text"
        })),
        _ => Err(SkillSdkError::new(
            "cli_command_unknown",
            format!("command={command}"),
        )),
    }
}

fn adopt_built_command(args: &[String]) -> Result<Value, SkillSdkError> {
    let outcome = SkillInstaller.adopt_built(&AdoptBuiltRequest {
        manifest_path: PathBuf::from(required_arg(args, 1, "manifest_path")?),
        workspace_root: PathBuf::from(required_arg(args, 2, "workspace_root")?),
        package_root: PathBuf::from(required_arg(args, 3, "package_root")?),
        binary_path: PathBuf::from(required_arg(args, 4, "binary_path")?),
        target: optional_flag_value(args, "--target").map(ToString::to_string),
        control: None,
    })?;
    Ok(json!({"ok": true, "command": "adopt-built", "install": outcome}))
}

fn init_command(args: &[String]) -> Result<Value, SkillSdkError> {
    let language = ImplementationLanguage::parse(required_arg(args, 1, "language")?)?;
    let skill_name = required_arg(args, 2, "skill_name")?;
    let destination = PathBuf::from(required_arg(args, 3, "destination")?);
    let outcome = scaffold_skill(&ScaffoldRequest {
        destination,
        skill_name: skill_name.to_string(),
        capability_summary: format!("Implement the {skill_name} capability through RustClaw."),
        actions: vec!["run".to_string()],
        implementation_language: language,
        source_root: ".".to_string(),
    })?;
    Ok(json!({"ok": true, "command": "init", "scaffold": outcome}))
}

fn validate_command(args: &[String]) -> Result<Value, SkillSdkError> {
    let path = required_arg(args, 1, "manifest_path")?;
    let manifest = PackageManifest::load(Path::new(path))?;
    let capability_request = manifest.effective_capability_request()?;
    Ok(json!({
        "ok": true,
        "command": "validate",
        "skill_name": manifest.package.name,
        "version": manifest.package.version,
        "manifest_schema_version": manifest.schema_version,
        "adapter": manifest.build.adapter.as_token(),
        "manifest_digest": manifest.digest()?,
        "semantic_contract_digest": manifest.capability_request_digest()?,
        "requested_capabilities": capability_request.capabilities.len(),
        "requested_permissions": capability_request.permissions,
    }))
}

fn install_command(command: &str, args: &[String]) -> Result<Value, SkillSdkError> {
    let manifest_path = PathBuf::from(required_arg(args, 1, "manifest_path")?);
    let workspace_root = PathBuf::from(required_arg(args, 2, "workspace_root")?);
    let package_root = PathBuf::from(required_arg(args, 3, "package_root")?);
    let allow_network = args.iter().any(|arg| arg == "--network");
    let target = optional_flag_value(args, "--target").map(ToString::to_string);
    let outcome = SkillInstaller.install(&InstallRequest {
        manifest_path,
        workspace_root,
        package_root,
        target,
        allow_network,
        control: None,
    })?;
    Ok(json!({"ok": true, "command": command, "install": outcome}))
}

fn package_command(args: &[String]) -> Result<Value, SkillSdkError> {
    let manifest_path = fs::canonicalize(required_arg(args, 1, "manifest_path")?)?;
    let output_root = PathBuf::from(required_arg(args, 2, "output_dir")?);
    let manifest = PackageManifest::load(&manifest_path)?;
    let source = manifest_path.parent().ok_or_else(|| {
        SkillSdkError::new(
            "manifest_parent_missing",
            manifest_path.display().to_string(),
        )
    })?;
    fs::create_dir_all(&output_root)?;
    let canonical_output = fs::canonicalize(&output_root)?;
    if canonical_output.starts_with(source) {
        return Err(SkillSdkError::new(
            "package_output_inside_source",
            canonical_output.display().to_string(),
        ));
    }
    let destination = canonical_output.join(format!(
        "{}-{}",
        manifest.package.name, manifest.package.version
    ));
    if destination.exists() {
        return Err(SkillSdkError::new(
            "package_destination_exists",
            destination.display().to_string(),
        ));
    }
    copy_source_tree(source, &destination)?;
    let index = json!({
        "schema_version": 1,
        "skill_name": manifest.package.name,
        "version": manifest.package.version,
        "manifest_digest": manifest.digest()?,
        "source_digest": source_digest(source)?,
    });
    fs::write(
        destination.join("package-index.json"),
        serde_json::to_vec_pretty(&index)?,
    )?;
    Ok(json!({"ok": true, "command": "package", "package_root": destination, "index": index}))
}

fn protocol_validate_command(args: &[String]) -> Result<Value, SkillSdkError> {
    let request_id = required_arg(args, 1, "request_id")?;
    let response_path = required_arg(args, 2, "response_path")?;
    let response = validate_response_line(&fs::read(response_path)?, request_id)?;
    Ok(json!({
        "ok": true,
        "command": "protocol-validate",
        "request_id": response.request_id,
        "status": response.status,
    }))
}

fn receipt_verify_command(args: &[String]) -> Result<Value, SkillSdkError> {
    let package_root = required_arg(args, 1, "package_root")?;
    let skill_name = required_arg(args, 2, "skill_name")?;
    let launch = SkillRuntimeResolver::new(package_root).resolve(skill_name)?;
    Ok(json!({"ok": true, "command": "receipt-verify", "launch": launch}))
}

fn rollback_command(args: &[String]) -> Result<Value, SkillSdkError> {
    let package_root = required_arg(args, 1, "package_root")?;
    let skill_name = required_arg(args, 2, "skill_name")?;
    let pointer = InstallReceiptStore::new(package_root).rollback(skill_name)?;
    Ok(json!({"ok": true, "command": "rollback", "current": pointer}))
}

fn human_summary(value: &Value) -> String {
    let command = value
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or("complete");
    let skill = value
        .pointer("/install/skill_name")
        .or_else(|| value.pointer("/scaffold/skill_name"))
        .or_else(|| value.get("skill_name"))
        .and_then(Value::as_str);
    match skill {
        Some(skill) => format!("{command}: {skill} ok"),
        None => format!("{command}: ok"),
    }
}

fn take_flag(args: &mut Vec<String>, flag: &str) -> bool {
    let present = args.iter().any(|value| value == flag);
    args.retain(|value| value != flag);
    present
}

fn required_arg<'a>(
    args: &'a [String],
    index: usize,
    name: &str,
) -> Result<&'a str, SkillSdkError> {
    args.get(index)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| SkillSdkError::new("cli_argument_missing", format!("argument={name}")))
}

fn optional_flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|value| value == flag)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
}
