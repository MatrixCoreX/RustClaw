pub(super) fn is_test_command_token(command: &str) -> bool {
    command_segments(command).any(|segment| is_test_command_segment(&segment))
}

pub(super) fn is_verification_command_token(command: &str) -> bool {
    command_segments(command).any(|segment| {
        is_test_command_segment(&segment)
            || segment.starts_with("cargo check")
            || segment.starts_with("cargo clippy")
            || segment.starts_with("cargo fmt")
            || segment.starts_with("npm run lint")
            || segment.starts_with("npm run build")
            || segment.starts_with("pnpm lint")
            || segment.starts_with("pnpm build")
            || segment.starts_with("yarn lint")
            || segment.starts_with("yarn build")
            || segment.starts_with("ruff check")
            || segment.starts_with("go vet")
    })
}

fn command_segments(command: &str) -> impl Iterator<Item = String> {
    command
        .trim()
        .to_ascii_lowercase()
        .split([';', '|'])
        .flat_map(|segment| segment.split("&&"))
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>()
        .into_iter()
}

fn is_test_command_segment(segment: &str) -> bool {
    segment.starts_with("cargo test")
        || segment.starts_with("npm test")
        || segment.starts_with("npm run test")
        || segment.starts_with("pnpm test")
        || segment.starts_with("yarn test")
        || segment.starts_with("pytest")
        || segment.starts_with("go test")
        || is_python_test_command_segment(segment)
}

fn is_python_test_command_segment(segment: &str) -> bool {
    let mut parts = segment.split_whitespace();
    let Some(program) = parts.next() else {
        return false;
    };
    let program = program.rsplit('/').next().unwrap_or(program);
    if program != "python"
        && program != "python3"
        && !program
            .strip_prefix("python3.")
            .is_some_and(|version| version.chars().all(|ch| ch.is_ascii_digit()))
    {
        return false;
    }
    let args = parts.collect::<Vec<_>>();
    if args.starts_with(&["-m", "pytest"]) || args.starts_with(&["-m", "unittest"]) {
        return true;
    }
    args.iter().any(|arg| {
        let arg = arg.trim_matches('"').trim_matches('\'');
        arg.starts_with("test_") || arg.ends_with("_test.py") || arg.contains("/test_")
    })
}
