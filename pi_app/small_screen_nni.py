import urllib.parse
from datetime import datetime
from decimal import Decimal, InvalidOperation


def nni_runtime_is_active(config):
    return bool(isinstance(config, dict) and config.get("joined") and config.get("worker_running", True))


def _node_host(config):
    if not isinstance(config, dict):
        return "--"
    host = str(config.get("last_success_node_host") or "").strip()
    if host:
        return host
    node_url = str(config.get("selected_node_url") or "").strip()
    if not node_url:
        nodes = config.get("remote_nodes")
        if isinstance(nodes, list) and nodes:
            node_url = str(nodes[0] or "").strip()
    try:
        return urllib.parse.urlparse(node_url).hostname or "--"
    except ValueError:
        return "--"


def _clock_label(timestamp):
    if not isinstance(timestamp, (int, float)) or timestamp <= 0:
        return "--"
    try:
        return datetime.fromtimestamp(timestamp).strftime("%m-%d %H:%M")
    except (OverflowError, OSError, ValueError):
        return "--"


def _reward_amount(value):
    try:
        parsed = Decimal(str(value).strip())
    except (InvalidOperation, TypeError, ValueError):
        return "--"
    if not parsed.is_finite() or parsed < 0:
        return "--"
    text = format(parsed, "f")
    if "." in text:
        text = text.rstrip("0").rstrip(".")
    return text or "0"


def nni_previous_window_metrics(rewards):
    if not isinstance(rewards, dict):
        return None, "--"
    network_devices = rewards.get("network_devices")
    network_devices = network_devices if isinstance(network_devices, dict) else {}
    active_count = network_devices.get("active_device_count")
    if not isinstance(active_count, int) or isinstance(active_count, bool) or active_count < 0:
        active_count = None

    network_rewards = rewards.get("network_rewards")
    network_rewards = network_rewards if isinstance(network_rewards, dict) else {}
    latest_period_end = network_rewards.get("latest_period_end_unix")
    if not isinstance(latest_period_end, int) or isinstance(latest_period_end, bool):
        latest_period_end = None
    records = rewards.get("records")
    records = records if isinstance(records, list) else []
    latest_record = max(
        (record for record in records if isinstance(record, dict)),
        key=lambda record: record.get("period_end_unix")
        if isinstance(record.get("period_end_unix"), int)
        else -1,
        default=None,
    )
    if latest_record is None:
        reward = "0" if latest_period_end is not None else "--"
    else:
        record_period_end = latest_record.get("period_end_unix")
        if latest_period_end is not None and record_period_end != latest_period_end:
            reward = "0"
        else:
            reward = _reward_amount(latest_record.get("reward_points"))
    return active_count, reward


def format_nni_runtime_summary(config, device, lang="CN", error="", rewards=None):
    selected_lang = "EN" if lang == "EN" else "CN"
    if not isinstance(config, dict) or not config:
        if error:
            return "NNI status unavailable" if selected_lang == "EN" else "NNI 状态暂时无法读取"
        return "Loading NNI status..." if selected_lang == "EN" else "正在读取 NNI 状态..."

    copy = {
        "CN": {
            "status": "状态",
            "running": "运行中",
            "stopped": "未启动",
            "heartbeat": "心跳",
            "active": "正常",
            "enabling": "启动中",
            "waiting_network": "等待网络",
            "degraded": "异常",
            "rejected": "已拒绝",
            "disabled": "未启用",
            "chip": "芯片",
            "hardware": "硬件",
            "simulated": "模拟",
            "unavailable": "不可用",
            "node": "节点",
            "authorization": "授权",
            "authorized": "已授权",
            "unknown": "待确认",
            "requests": "请求",
            "latest": "最近",
            "network_active": "全网活跃设备",
            "previous_reward": "上一窗口奖励",
            "sync_error": "状态同步失败，保留上次结果",
        },
        "EN": {
            "status": "Status",
            "running": "Running",
            "stopped": "Stopped",
            "heartbeat": "Heartbeat",
            "active": "Active",
            "enabling": "Starting",
            "waiting_network": "Waiting for network",
            "degraded": "Degraded",
            "rejected": "Rejected",
            "disabled": "Disabled",
            "chip": "Chip",
            "hardware": "Hardware",
            "simulated": "Simulated",
            "unavailable": "Unavailable",
            "node": "Node",
            "authorization": "Authorization",
            "authorized": "Authorized",
            "unknown": "Pending",
            "requests": "Requests",
            "latest": "Latest",
            "network_active": "Network active devices",
            "previous_reward": "Previous-window reward",
            "sync_error": "Status refresh failed; showing the last result",
        },
    }[selected_lang]

    active = nni_runtime_is_active(config)
    heartbeat_state = str(config.get("heartbeat_state") or ("enabling" if active else "disabled"))
    heartbeat_label = copy.get(heartbeat_state, heartbeat_state)
    device = device if isinstance(device, dict) else {}
    if device.get("hardware_chip_present") or (
        device.get("signature_chip_present") and not device.get("simulated")
    ):
        chip_label = copy["hardware"]
    elif device.get("simulated"):
        chip_label = copy["simulated"]
    else:
        chip_label = copy["unavailable"]
    authorization = str(config.get("network_authorization") or "unknown")
    authorization_label = copy.get(authorization, authorization)
    request_count = config.get("heartbeat_request_count")
    if not isinstance(request_count, int) or isinstance(request_count, bool):
        request_count = 0

    lines = [
        f"{copy['status']}: {copy['running'] if active else copy['stopped']}  ·  "
        f"{copy['heartbeat']}: {heartbeat_label}  ·  {copy['chip']}: {chip_label}",
        f"{copy['node']}: {_node_host(config)}  ·  {copy['authorization']}: {authorization_label}",
        f"{copy['requests']}: {request_count}  ·  "
        f"{copy['latest']}: {_clock_label(config.get('last_heartbeat_at_ts'))}",
    ]
    active_count, previous_reward = nni_previous_window_metrics(rewards)
    lines.append(
        f"{copy['network_active']}: {active_count if active_count is not None else '--'}  ·  "
        f"{copy['previous_reward']}: {previous_reward}"
    )
    if error:
        lines.append(copy["sync_error"])
    return "\n".join(lines)
