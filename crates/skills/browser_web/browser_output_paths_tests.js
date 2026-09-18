const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const { initCaptureStorage, resolveOutputPath } = require('./browser_web.js');

async function fixture(t) {
    const root = await fs.mkdtemp(path.join(os.tmpdir(), 'browser-output-'));
    t.after(() => fs.rm(root, { recursive: true, force: true }));
    const workspace = path.join(root, 'workspace');
    const artifact = path.join(root, 'artifacts', 'invocation-1');
    await fs.mkdir(workspace, { recursive: true });
    await fs.mkdir(artifact, { recursive: true });
    return { root, workspace, artifact };
}

test('capture storage accepts the host-issued artifact directory outside the workspace', async (t) => {
    const { workspace, artifact } = await fixture(t);
    const capture = await initCaptureStorage({
        workspaceRoot: workspace,
        artifactOutputDirectory: artifact,
        captureRoot: path.join(artifact, 'capture'),
        runId: 'bounded-test',
    });
    assert.equal(capture.enabled, true);
    assert.equal(capture.runRoot.startsWith(`${artifact}${path.sep}`), true);
    assert.equal((await fs.stat(capture.files.manifest)).isFile(), true);
    assert.equal(resolveOutputPath(workspace, path.join(artifact, 'screenshots', 'page.png'), {
        artifactOutputDirectory: artifact,
    }), path.join(artifact, 'screenshots', 'page.png'));
});

test('an artifact grant does not allow sibling paths, traversal or arbitrary outside output', async (t) => {
    const { root, workspace, artifact } = await fixture(t);
    for (const target of [
        path.join(root, 'outside', 'page.png'),
        path.join(artifact, '..', 'invocation-2', 'page.png'),
        `${artifact}-other/page.png`,
    ]) {
        assert.throws(() => resolveOutputPath(workspace, target, {
            artifactOutputDirectory: artifact,
        }), { code: 'WORKSPACE_PATH_OUTSIDE' });
    }
    await assert.rejects(initCaptureStorage({
        workspaceRoot: workspace,
        captureRoot: path.join(artifact, 'capture'),
    }), { code: 'WORKSPACE_PATH_OUTSIDE' });
});

test('output checks canonical ancestors before creating children through a symlink', async (t) => {
    const { root, workspace, artifact } = await fixture(t);
    const outside = path.join(root, 'outside');
    await fs.mkdir(outside);
    for (const base of [workspace, artifact]) {
        await fs.symlink(outside, path.join(base, 'escape'), 'dir');
        assert.throws(() => resolveOutputPath(workspace, path.join(base, 'escape', 'new', 'page.png'), {
            artifactOutputDirectory: artifact,
        }), { code: 'WORKSPACE_PATH_OUTSIDE' });
    }
    await assert.rejects(fs.stat(path.join(outside, 'new')), { code: 'ENOENT' });
});

test('ordinary workspace output and explicit outside-workspace grants remain supported', async (t) => {
    const { root, workspace } = await fixture(t);
    const inside = path.join(workspace, 'captures', 'page.png');
    assert.equal(resolveOutputPath(workspace, inside), inside);
    const outside = path.join(root, 'explicit', 'page.png');
    assert.equal(resolveOutputPath(workspace, outside, { allowPathOutsideWorkspace: true }), outside);
    assert.throws(() => resolveOutputPath(workspace, outside, {
        artifactOutputDirectory: '../explicit',
    }), { code: 'WORKSPACE_PATH_OUTSIDE' });
});
