import {spawnSync} from 'node:child_process';
const checks = [['rustc', ['--version']], ['cargo', ['--version']], [process.execPath, ['--version']]];
if (process.platform === 'linux') checks.push(['pkg-config', ['--modversion', 'gtk+-3.0', 'webkit2gtk-4.1']]);
else if (process.platform === 'darwin') checks.push(['xcrun', ['--show-sdk-path']], ['xcodebuild', ['-version']]);
else if (process.platform === 'win32') {
  if (process.arch !== 'x64') throw new Error('desktop_windows_build_requires_x64_host');
  checks.push(['rustc', ['-vV']]);
  console.log('Windows also requires Visual Studio C++ Build Tools, Windows SDK and WebView2.');
} else throw new Error('desktop_build_platform_unsupported');
for (const [command, args] of checks) {
  const result = spawnSync(command, args, {stdio: 'inherit'});
  if (result.error || result.status !== 0) throw new Error(`desktop_build_dependency_unavailable: ${command}`);
}
