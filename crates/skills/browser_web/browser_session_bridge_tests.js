'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs/promises');
const path = require('path');
const readline = require('readline');
const { spawn } = require('child_process');
const { once } = require('events');

const {
    checkPostcondition,
    detectMediaType,
    redactText,
    safeUrl,
} = require('./browser_session_bridge.js');

test('session projection helpers remove URL secrets and diagnostic credentials', () => {
    assert.equal(
        safeUrl('https://user:pass@example.com/path?token=secret#fragment'),
        'https://example.com/path',
    );
    const redacted = redactText('Bearer abc.def.ghi https://x.test/?secret=value ghp_1234567890abcdef');
    assert.equal(redacted.includes('abc.def.ghi'), false);
    assert.equal(redacted.includes('value'), false);
    assert.equal(redacted.includes('ghp_1234567890abcdef'), false);
});

test('download media type uses bounded content sniffing before filename fallback', () => {
    assert.equal(detectMediaType(Buffer.from('%PDF-1.7\n'), 'wrong.bin'), 'application/pdf');
    assert.equal(detectMediaType(Buffer.from('{"ok":true}'), 'result.json'), 'application/json');
    assert.equal(detectMediaType(Buffer.from([0, 1, 2, 3]), 'unknown.bin'), 'application/octet-stream');
});

test('postconditions use structured observations only', () => {
    const before = { current_url: 'https://example.com/a', title: 'A', page_count: 1 };
    const after = {
        current_url: 'https://example.com/b', title: 'B', tabs: [{ page_id: 'p1' }, { page_id: 'p2' }],
    };
    assert.deepEqual(checkPostcondition({ kind: 'url_changed' }, before, after), {
        status: 'observed', kind: 'url_changed',
    });
    assert.deepEqual(checkPostcondition({ kind: 'page_count_changed' }, before, after), {
        status: 'observed', kind: 'page_count_changed',
    });
});

test('versioned bridge opens, snapshots, rejects stale page generations, writes artifact, and exits', {
    timeout: 60_000,
}, async (t) => {
    const workspaceRoot = process.cwd();
    const artifactRoot = await fs.mkdtemp(path.join(workspaceRoot, '.browser-session-test-'));
    const child = spawn(process.execPath, [path.join(__dirname, 'browser_session_bridge.js')], {
        cwd: workspaceRoot,
        stdio: ['pipe', 'pipe', 'pipe'],
        env: { ...process.env, BROWSER_SESSION_BRIDGE_TEST_MODE: '1' },
    });
    let stderr = '';
    child.stderr.setEncoding('utf8');
    child.stderr.on('data', (chunk) => { stderr += chunk; });
    const lines = readline.createInterface({ input: child.stdout, crlfDelay: Infinity })[Symbol.asyncIterator]();
    let requestSequence = 0;
    const request = async (command) => {
        const requestId = `test-${++requestSequence}`;
        child.stdin.write(`${JSON.stringify({ schema_version: 1, request_id: requestId, ...command })}\n`);
        let timer;
        let next;
        try {
            next = await Promise.race([
                lines.next(),
                new Promise((_, reject) => {
                    timer = setTimeout(
                        () => reject(new Error(`bridge request timed out: ${command.command}; stderr=${stderr}`)),
                        15_000,
                    );
                }),
            ]);
        } finally {
            clearTimeout(timer);
        }
        assert.equal(next.done, false, `bridge exited early: ${stderr}`);
        const response = JSON.parse(next.value);
        assert.equal(response.request_id, requestId);
        return response;
    };
    t.after(async () => {
        child.kill('SIGKILL');
        await fs.rm(artifactRoot, { recursive: true, force: true });
    });

    const opened = await request({
        command: 'session_open',
        workspace_root: workspaceRoot,
        artifact_root: artifactRoot,
        locale: 'zh-CN',
        timezone: 'Asia/Shanghai',
    });
    assert.equal(opened.status, 'ok');
    assert.equal(opened.result.snapshot.trust.classification, 'untrusted_web_content');
    assert.equal(opened.result.snapshot.locale, 'zh-CN');
    assert.equal(opened.result.snapshot.timezone, 'Asia/Shanghai');
    const { page_id: pageId, page_generation: pageGeneration } = opened.result;

    const privateNavigation = await request({
        command: 'navigate', page_id: pageId, expected_page_generation: pageGeneration,
        url: 'http://127.0.0.1/private',
    });
    assert.equal(privateNavigation.status, 'error');
    assert.equal(privateNavigation.error_code, 'PRIVATE_NETWORK_BLOCKED');

    const loaded = await request({
        command: 'test_fixture_load', page_id: pageId, expected_page_generation: pageGeneration,
    });
    assert.equal(loaded.status, 'ok');
    const activeGeneration = loaded.result.page_generation;
    const findElement = (snapshot, name) => snapshot.elements.find((element) => element.name === name);
    let currentSnapshot = loaded.result;
    assert.equal(currentSnapshot.challenge.authentication_fields_present, false);
    assert.equal(currentSnapshot.frames.length >= 2, true);
    assert.equal(currentSnapshot.elements.filter((element) => element.name === 'Duplicate').length, 2);
    const originalSnapshotId = currentSnapshot.snapshot_id;
    const originalApplyRef = findElement(currentSnapshot, 'Apply').ref;

    const uploadType = await request({
        command: 'type', page_id: pageId, expected_page_generation: activeGeneration,
        expected_snapshot_id: currentSnapshot.snapshot_id,
        target_ref: findElement(currentSnapshot, 'Upload').ref,
        value: '/tmp/secret',
    });
    assert.equal(uploadType.status, 'error');
    assert.equal(uploadType.error_code, 'FILE_UPLOAD_UNSUPPORTED');

    const typed = await request({
        command: 'type', page_id: pageId, expected_page_generation: activeGeneration,
        expected_snapshot_id: currentSnapshot.snapshot_id,
        target_ref: findElement(currentSnapshot, 'Query').ref,
        value: 'runtime',
    });
    assert.equal(typed.status, 'ok');
    assert.equal(typed.result.action_metadata.value_redacted, true);
    assert.equal(typed.result.action_metadata.value_length, 7);
    currentSnapshot = typed.result.snapshot;

    const pressed = await request({
        command: 'press_key', page_id: pageId, expected_page_generation: activeGeneration,
        expected_snapshot_id: currentSnapshot.snapshot_id,
        target_ref: findElement(currentSnapshot, 'Query').ref,
        key: 'Enter',
    });
    assert.equal(pressed.status, 'ok');
    assert.match(pressed.result.snapshot.visible_text, /Keyboard applied/);
    currentSnapshot = pressed.result.snapshot;

    const checked = await request({
        command: 'click', page_id: pageId, expected_page_generation: activeGeneration,
        expected_snapshot_id: currentSnapshot.snapshot_id,
        target_ref: findElement(currentSnapshot, 'Include archived').ref,
    });
    assert.equal(checked.status, 'ok');
    assert.equal(findElement(checked.result.snapshot, 'Include archived').checked, true);
    currentSnapshot = checked.result.snapshot;

    const selected = await request({
        command: 'select', page_id: pageId, expected_page_generation: activeGeneration,
        expected_snapshot_id: currentSnapshot.snapshot_id,
        target_ref: findElement(currentSnapshot, 'Category').ref,
        value: 'docs',
    });
    assert.equal(selected.status, 'ok');
    currentSnapshot = selected.result.snapshot;

    const clicked = await request({
        command: 'click', page_id: pageId, expected_page_generation: activeGeneration,
        expected_snapshot_id: currentSnapshot.snapshot_id,
        target_ref: findElement(currentSnapshot, 'Apply').ref,
    });
    assert.equal(clicked.status, 'ok');
    assert.equal(clicked.result.observed_effect, 'state_changed');
    assert.match(clicked.result.snapshot.visible_text, /Applied runtime in docs/);
    currentSnapshot = clicked.result.snapshot;

    const scrolled = await request({
        command: 'scroll', page_id: pageId, expected_page_generation: activeGeneration,
        delta_y: 300,
    });
    assert.equal(scrolled.status, 'ok');
    assert.equal(scrolled.result.retry_safe, true);
    currentSnapshot = scrolled.result.snapshot;

    const waited = await request({
        command: 'wait_for', page_id: pageId, expected_page_generation: activeGeneration,
        condition: 'load', timeout_ms: 1000,
    });
    assert.equal(waited.status, 'ok');
    assert.equal(waited.result.retry_safe, true);
    currentSnapshot = waited.result.snapshot;

    const firstTextPage = await request({
        command: 'snapshot', page_id: pageId, expected_page_generation: activeGeneration,
        max_text_chars: 100,
    });
    assert.equal(firstTextPage.status, 'ok');
    assert.equal(firstTextPage.result.text_page.has_more, true);
    const secondTextPage = await request({
        command: 'snapshot', page_id: pageId, expected_page_generation: activeGeneration,
        max_text_chars: 100,
        text_cursor: firstTextPage.result.text_page.next_cursor,
    });
    assert.equal(secondTextPage.status, 'ok');
    assert.notEqual(secondTextPage.result.visible_text, firstTextPage.result.visible_text);
    currentSnapshot = secondTextPage.result;

    const staleRef = await request({
        command: 'click', page_id: pageId, expected_page_generation: activeGeneration,
        expected_snapshot_id: originalSnapshotId,
        target_ref: originalApplyRef,
    });
    assert.equal(staleRef.status, 'error');
    assert.equal(staleRef.error_code, 'STALE_SNAPSHOT_REF');

    const popup = await request({
        command: 'click', page_id: pageId, expected_page_generation: activeGeneration,
        expected_snapshot_id: currentSnapshot.snapshot_id,
        target_ref: findElement(currentSnapshot, 'Open details tab').ref,
        expected_postcondition: { kind: 'page_count_changed' },
    });
    assert.equal(popup.status, 'ok');
    assert.equal(popup.result.postcondition_status, 'observed');
    assert.equal(popup.result.snapshot.tabs.length, 2);
    currentSnapshot = popup.result.snapshot;
    const popupTab = currentSnapshot.tabs.find((tab) => tab.page_id !== pageId);
    assert.ok(popupTab);
    const switched = await request({
        command: 'switch_page', page_id: popupTab.page_id,
        expected_page_generation: popupTab.page_generation,
    });
    assert.equal(switched.status, 'ok');
    assert.match(switched.result.visible_text, /Popup evidence/);

    const downloaded = await request({
        command: 'download', page_id: pageId, expected_page_generation: activeGeneration,
        expected_snapshot_id: currentSnapshot.snapshot_id,
        target_ref: findElement(currentSnapshot, 'Download report').ref,
    });
    assert.equal(downloaded.status, 'ok');
    assert.equal(downloaded.result.download.media_type, 'text/plain');
    assert.equal(downloaded.result.download.suggested_filename, 'fixture-report.txt');
    assert.equal((await fs.readFile(downloaded.result.download.path, 'utf8')), 'fixture report\n');

    const debug = await request({
        command: 'observe_debug', page_id: pageId, expected_page_generation: activeGeneration,
    });
    assert.equal(debug.status, 'ok');
    assert.equal(debug.result.request_headers_included, false);
    assert.equal(debug.result.request_bodies_included, false);
    assert.equal(debug.result.page_summary.raw_html_included, false);
    assert.equal(JSON.stringify(debug.result).includes('secret-console-token'), false);
    assert.equal(JSON.stringify(debug.result).includes('secret-query'), false);

    const authentication = await request({
        command: 'test_fixture_load', fixture: 'auth', page_id: pageId,
        expected_page_generation: activeGeneration,
    });
    assert.equal(authentication.status, 'ok');
    assert.equal(authentication.result.challenge.authentication_fields_present, true);
    assert.equal(authentication.result.challenge.automation_allowed, false);
    const credentialType = await request({
        command: 'type', page_id: pageId,
        expected_page_generation: authentication.result.page_generation,
        expected_snapshot_id: authentication.result.snapshot_id,
        target_ref: findElement(authentication.result, 'Password').ref,
        value: 'must-not-be-entered',
    });
    assert.equal(credentialType.status, 'error');
    assert.equal(credentialType.error_code, 'BROWSER_AUTHENTICATION_REQUIRES_USER');

    const challenge = await request({
        command: 'test_fixture_load', fixture: 'captcha', page_id: pageId,
        expected_page_generation: authentication.result.page_generation,
    });
    assert.equal(challenge.status, 'ok');
    assert.equal(challenge.result.challenge.captcha_detected, true);
    const challengeClick = await request({
        command: 'click', page_id: pageId,
        expected_page_generation: challenge.result.page_generation,
        expected_snapshot_id: challenge.result.snapshot_id,
        target_ref: findElement(challenge.result, 'Continue').ref,
    });
    assert.equal(challengeClick.status, 'error');
    assert.equal(challengeClick.error_code, 'BROWSER_CHALLENGE_REQUIRES_USER');

    const stale = await request({
        command: 'snapshot', page_id: pageId,
        expected_page_generation: challenge.result.page_generation + 1,
    });
    assert.equal(stale.status, 'error');
    assert.equal(stale.error_code, 'STALE_PAGE_GENERATION');
    assert.equal(stale.retryable, false);

    const screenshot = await request({
        command: 'screenshot', page_id: pageId,
        expected_page_generation: challenge.result.page_generation,
    });
    assert.equal(screenshot.status, 'ok');
    assert.equal(screenshot.result.artifact.media_type, 'image/png');
    const screenshotPath = screenshot.result.artifact.path;
    assert.equal(path.resolve(screenshotPath).startsWith(`${path.resolve(artifactRoot)}${path.sep}`), true);
    assert.equal((await fs.stat(screenshotPath)).size > 0, true);

    const exited = once(child, 'exit');
    const closed = await request({ command: 'session_close' });
    assert.equal(closed.status, 'ok');
    assert.equal(closed.result.orphan_processes_expected, false);
    const [exitCode, signal] = await exited;
    assert.equal(exitCode, 0, `signal=${signal} stderr=${stderr}`);
});
