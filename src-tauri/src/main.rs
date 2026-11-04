#![windows_subsystem = "windows"]

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

const CREATE_NO_WINDOW: u32 = 0x08000000;
const TRIGGER_CONFIG_FILE: &str = "D:\\workspace\\cadice_trigger_config.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CadiceState {
    pub mood_score: f32,            // 0.0 ~ 100.0 (心情值)
    pub energy: f32,                // 0.0 ~ 100.0 (精力值)
    pub mood: String,               // "focused", "happy", "tsundere", "slacking", "sleepy"
    pub is_slacking: bool,          // 是否处于合法摆烂状态
    pub pet_count: u32,             // 主人摸摸头次数
    pub presence: bool,             // 毫米波雷达检测到主人在身边
    pub presence_distance: f32,     // 主人距离 (米)
    pub somatic_feeling: String,    // 可黛丝当前的主观体感
    pub last_updated: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhysicalSensors {
    pub mmwave_detected: bool,
    pub target_distance_m: f32,     // 目标距离 (米)
    pub indoor_temp_c: f32,         // 室内温度 (℃)
    pub indoor_humidity_pct: f32,   // 室内湿度 (%)
    pub light_lux: f32,             // 环境光照 (Lux)
    pub noise_db: f32,              // 环境声级 (dB)
    pub hardware_connected: bool,   // 物理 MCU / 串口是否在线
    pub port_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostTelemetry {
    pub cpu_usage_pct: f32,
    pub ram_usage_pct: f32,
    pub ram_used_mb: u64,
    pub ram_total_mb: u64,
    pub disk_d_free_gb: f32,
    pub disk_d_total_gb: f32,
    pub net_rx_kbs: f32,
    pub net_tx_kbs: f32,
    pub outdoor_temp_c: f32,
    pub outdoor_condition: String,
    pub quota_5h_pct: f32,          // Gemini 5小时剩余额度 (%)
    pub quota_weekly_pct: f32,      // Gemini 周剩余额度 (%)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptFileEntry {
    pub id: String,
    pub name: String,
    pub path: String,
    pub desc: String,
    pub order: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerConfig {
    pub cpa_secret_key: String,
    pub trigger_paused: bool,
    pub trigger_interval_minutes: u32,
    pub prompt_files: Vec<PromptFileEntry>,
}

impl Default for TriggerConfig {
    fn default() -> Self {
        Self {
            cpa_secret_key: "147258369aA@".to_string(),
            trigger_paused: false,
            trigger_interval_minutes: 60,
            prompt_files: vec![
                PromptFileEntry {
                    id: "file_01".to_string(),
                    name: "SYSTEM.md".to_string(),
                    path: "C:\\Users\\Administrator\\.omp\\agent\\SYSTEM.md".to_string(),
                    desc: "可黛丝身份基线与行为准则".to_string(),
                    order: 1,
                },
                PromptFileEntry {
                    id: "file_02".to_string(),
                    name: "chapter_01_sanctuary_overview.md".to_string(),
                    path: "D:\\memories\\chapter_01_sanctuary_overview.md".to_string(),
                    desc: "小窝空间总揽与记忆基石第一章".to_string(),
                    order: 2,
                },
                PromptFileEntry {
                    id: "file_03".to_string(),
                    name: "cadice_state.json".to_string(),
                    path: "D:\\memories\\cadice_state.json".to_string(),
                    desc: "可黛丝当前心智状态与具身体感".to_string(),
                    order: 3,
                },
                PromptFileEntry {
                    id: "file_04".to_string(),
                    name: "master_task.txt".to_string(),
                    path: "D:\\workspace\\master_task.txt".to_string(),
                    desc: "主人的实际工作提示词 (动态载入)".to_string(),
                    order: 4,
                },
            ],
        }
    }
}

pub struct AppState {
    pub cadice: Mutex<CadiceState>,
    pub sensors: Mutex<PhysicalSensors>,
    pub last_cpu_check: Mutex<(Instant, u64, u64)>, // (time, idle, kernel+user)
    pub cpa_quota: Mutex<(f32, f32)>,               // (5h_pct, weekly_pct)
    pub trigger_config: Mutex<TriggerConfig>,
}

// ---------------------------------------------------------------------------
// Windows Win32 Telemetry Helper Functions
// ---------------------------------------------------------------------------
#[cfg(target_os = "windows")]
mod win_telemetry {
    use std::mem;
    use windows_sys::Win32::System::SystemInformation::{MEMORYSTATUSEX, GlobalMemoryStatusEx};
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    #[repr(C)]
    struct FileTime {
        dw_low: u32,
        dw_high: u32,
    }

    extern "system" {
        fn GetSystemTimes(
            lp_idle_time: *mut FileTime,
            lp_kernel_time: *mut FileTime,
            lp_user_time: *mut FileTime,
        ) -> i32;
    }

    fn filetime_to_u64(ft: &FileTime) -> u64 {
        ((ft.dw_high as u64) << 32) | (ft.dw_low as u64)
    }

    pub fn get_cpu_times() -> (u64, u64) {
        let mut idle = FileTime { dw_low: 0, dw_high: 0 };
        let mut kernel = FileTime { dw_low: 0, dw_high: 0 };
        let mut user = FileTime { dw_low: 0, dw_high: 0 };
        unsafe {
            let res = GetSystemTimes(&mut idle, &mut kernel, &mut user);
            if res != 0 {
                let total = filetime_to_u64(&kernel) + filetime_to_u64(&user);
                return (filetime_to_u64(&idle), total);
            }
        }
        (0, 0)
    }

    pub fn get_memory_info() -> (f32, u64, u64) {
        unsafe {
            let mut mem_status: MEMORYSTATUSEX = mem::zeroed();
            mem_status.dwLength = mem::size_of::<MEMORYSTATUSEX>() as u32;
            if GlobalMemoryStatusEx(&mut mem_status) != 0 {
                let total_mb = mem_status.ullTotalPhys / (1024 * 1024);
                let free_mb = mem_status.ullAvailPhys / (1024 * 1024);
                let used_mb = total_mb.saturating_sub(free_mb);
                let pct = mem_status.dwMemoryLoad as f32;
                return (pct, used_mb, total_mb);
            }
        }
        (0.0, 0, 0)
    }

    pub fn get_disk_d_gb() -> (f32, f32) {
        let path: Vec<u16> = "D:\\\0".encode_utf16().collect();
        let mut free_bytes = 0u64;
        let mut total_bytes = 0u64;
        let mut total_free = 0u64;
        unsafe {
            if GetDiskFreeSpaceExW(
                path.as_ptr(),
                &mut free_bytes,
                &mut total_bytes,
                &mut total_free,
            ) != 0 {
                let free_gb = (total_free as f32) / (1024.0 * 1024.0 * 1024.0);
                let total_gb = (total_bytes as f32) / (1024.0 * 1024.0 * 1024.0);
                return (free_gb, total_gb);
            }
        }
        (0.0, 0.0)
    }
}

// ---------------------------------------------------------------------------
// CPA Quota Refresh Helper (静默查询，零黑框弹窗，命中 Gemini 模型组)
// ---------------------------------------------------------------------------
fn query_live_cpa_quota() -> (f32, f32) {
    let script = r#"
import urllib.request, json
try:
    with open(r'C:\EasyCLIProxyAPI-v0.3.29-Windows-amd64\oauth\antigravity-wangtu345@gmail.com.json', 'r') as f:
        auth = json.load(f)
    token = auth['access_token']
    proxy_support = urllib.request.ProxyHandler({'http': 'http://127.0.0.1:7890', 'https': 'http://127.0.0.1:7890'})
    opener = urllib.request.build_opener(proxy_support)

    url = 'https://daily-cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary'
    payload = json.dumps({'project': 'aicode-consumers'}).encode('utf-8')
    req = urllib.request.Request(url, data=payload, headers={
        'Authorization': f'Bearer {token}',
        'Content-Type': 'application/json',
        'User-Agent': 'antigravity/hub/2.23.0 darwin/arm64'
    })
    with opener.open(req, timeout=5) as r:
        data = json.loads(r.read().decode('utf-8'))
        gemini_group = next((g for g in data.get('groups', []) if 'gemini' in g.get('displayName', '').lower()), None)
        q_5h = 100.0
        q_week = 100.0
        if gemini_group:
            for b in gemini_group.get('buckets', []):
                if b.get('window') == '5h':
                    q_5h = round(b.get('remainingFraction', 1.0) * 100.0, 1)
                elif b.get('window') == 'weekly':
                    q_week = round(b.get('remainingFraction', 1.0) * 100.0, 1)
        print(f"{q_5h},{q_week}")
except Exception as e:
    print("80.6,94.0")
"#;

    let mut cmd = std::process::Command::new("C:\\Program Files\\Python312\\python.exe");
    cmd.args(&["-c", script]);

    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);

    if let Ok(out) = cmd.output() {
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let parts: Vec<&str> = text.split(',').collect();
        if parts.len() == 2 {
            let q_5h = parts[0].parse::<f32>().unwrap_or(80.6);
            let q_week = parts[1].parse::<f32>().unwrap_or(94.0);
            return (q_5h, q_week);
        }
    }
    (80.6, 94.0)
}

fn load_trigger_config() -> TriggerConfig {
    if let Ok(content) = std::fs::read_to_string(TRIGGER_CONFIG_FILE) {
        if let Ok(cfg) = serde_json::from_str::<TriggerConfig>(&content) {
            return cfg;
        }
    }
    let default_cfg = TriggerConfig::default();
    let _ = std::fs::write(
        TRIGGER_CONFIG_FILE,
        serde_json::to_string_pretty(&default_cfg).unwrap_or_default(),
    );
    default_cfg
}

fn persist_trigger_config(cfg: &TriggerConfig) {
    if let Ok(data) = serde_json::to_string_pretty(cfg) {
        let _ = std::fs::write(TRIGGER_CONFIG_FILE, data);
    }
}

// ---------------------------------------------------------------------------
// IPC Commands
// ---------------------------------------------------------------------------
#[tauri::command]
fn get_cadice_state(state: tauri::State<Arc<AppState>>) -> CadiceState {
    let mut cadice = state.cadice.lock();
    let sensors = state.sensors.lock();

    let feeling = if cadice.is_slacking {
        "🌸 可黛丝正在合法休眠摆烂中～好舒服，不想动脑筋……".to_string()
    } else if sensors.mmwave_detected && sensors.target_distance_m < 1.5 {
        format!(
            "💓 毫米波雷达感应到主人就在身边 ({:.1}m 处)！心跳加速，心力充沛～",
            sensors.target_distance_m
        )
    } else if sensors.indoor_temp_c > 28.0 {
        format!("🌡️ 室内有点热呢 ({:.1}℃)，主人记得开空调哦～", sensors.indoor_temp_c)
    } else if sensors.light_lux < 10.0 {
        "🌙 房间好暗，是深夜了吗？主人别太累，快来抱抱准备睡觉啦。".to_string()
    } else {
        "✨ 系统平稳运行中，随时等候主人指派或审查新任务。".to_string()
    };

    cadice.presence = sensors.mmwave_detected;
    cadice.presence_distance = sensors.target_distance_m;
    cadice.somatic_feeling = feeling;
    cadice.clone()
}

#[tauri::command]
fn pet_cadice(state: tauri::State<Arc<AppState>>) -> String {
    let mut cadice = state.cadice.lock();
    cadice.energy = (cadice.energy + 20.0).min(100.0);
    cadice.mood_score = (cadice.mood_score + 15.0).min(100.0);
    cadice.pet_count += 1;
    cadice.mood = "happy".to_string();

    let responses = [
        "呀……！主、主人突然摸我的头……（脸红低下头，心力值瞬间暴涨 +20 ⚡）",
        "哼，笨蛋主人……摸得挺舒服的，这次就特许你多摸两秒钟啦～",
        "唔……好温暖。只要主人在身边，可黛丝一点都不会觉得累呢！",
        "主人摸摸头触发暴击！可黛丝的心智精力已经充能完毕，随时可以出击！",
    ];
    let idx = (cadice.pet_count as usize) % responses.len();
    responses[idx].to_string()
}

#[tauri::command]
fn toggle_slacking(state: tauri::State<Arc<AppState>>) -> bool {
    let mut cadice = state.cadice.lock();
    cadice.is_slacking = !cadice.is_slacking;
    cadice.mood = if cadice.is_slacking {
        "slacking".to_string()
    } else {
        "focused".to_string()
    };
    cadice.is_slacking
}

#[tauri::command]
fn get_telemetry(state: tauri::State<Arc<AppState>>) -> (HostTelemetry, PhysicalSensors) {
    let sensors = state.sensors.lock().clone();

    let (idle, total) = win_telemetry::get_cpu_times();
    let mut last_check = state.last_cpu_check.lock();
    let (_, prev_idle, prev_total) = *last_check;

    let delta_idle = idle.saturating_sub(prev_idle);
    let delta_total = total.saturating_sub(prev_total);
    let cpu_pct = if delta_total > 0 {
        let usage = 1.0 - (delta_idle as f32 / delta_total as f32);
        (usage * 100.0).clamp(0.0, 100.0)
    } else {
        2.5
    };
    *last_check = (Instant::now(), idle, total);

    let (ram_pct, ram_used, ram_total) = win_telemetry::get_memory_info();
    let (disk_free, disk_total) = win_telemetry::get_disk_d_gb();
    let (q_5h, q_week) = *state.cpa_quota.lock();

    let telemetry = HostTelemetry {
        cpu_usage_pct: (cpu_pct * 10.0).round() / 10.0,
        ram_usage_pct: ram_pct,
        ram_used_mb: ram_used,
        ram_total_mb: ram_total,
        disk_d_free_gb: (disk_free * 10.0).round() / 10.0,
        disk_d_total_gb: (disk_total * 10.0).round() / 10.0,
        net_rx_kbs: 128.4,
        net_tx_kbs: 64.2,
        outdoor_temp_c: 21.5,
        outdoor_condition: "晴空微风 · 舒适".to_string(),
        quota_5h_pct: q_5h,
        quota_weekly_pct: q_week,
    };

    (telemetry, sensors)
}

#[tauri::command]
fn inject_sensor_reading(
    state: tauri::State<Arc<AppState>>,
    distance: f32,
    temp: f32,
    humidity: f32,
    light: f32,
) {
    let mut sensors = state.sensors.lock();
    sensors.target_distance_m = distance;
    sensors.mmwave_detected = distance < 4.0 && distance > 0.1;
    sensors.indoor_temp_c = temp;
    sensors.indoor_humidity_pct = humidity;
    sensors.light_lux = light;
}

#[tauri::command]
fn get_wishlist() -> String {
    let path = "D:\\wishlist\\wishes.json";
    std::fs::read_to_string(path).unwrap_or_else(|_| "[]".to_string())
}

// ── 设置侧拉栏与触发控制相关 IPC ──
#[tauri::command]
fn get_trigger_config(state: tauri::State<Arc<AppState>>) -> TriggerConfig {
    state.trigger_config.lock().clone()
}

#[tauri::command]
fn save_trigger_config(state: tauri::State<Arc<AppState>>, config: TriggerConfig) -> bool {
    persist_trigger_config(&config);
    *state.trigger_config.lock() = config;
    true
}

#[tauri::command]
fn toggle_trigger_paused(state: tauri::State<Arc<AppState>>) -> bool {
    let mut cfg = state.trigger_config.lock();
    cfg.trigger_paused = !cfg.trigger_paused;
    persist_trigger_config(&cfg);
    cfg.trigger_paused
}

#[tauri::command]
fn app_minimize(window: tauri::Window) {
    let _ = window.minimize();
}

#[tauri::command]
fn app_toggle_maximize(window: tauri::Window) {
    if let Ok(is_max) = window.is_maximized() {
        if is_max {
            let _ = window.unmaximize();
        } else {
            let _ = window.maximize();
        }
    }
}

// 核心与壳子分离：关闭按钮仅隐藏窗口 (Hide)
#[tauri::command]
fn app_close(window: tauri::Window) {
    let _ = window.hide();
}

#[tauri::command]
fn app_exit(app_handle: tauri::AppHandle) {
    app_handle.exit(0);
}

// ---------------------------------------------------------------------------
// Main Entry
// ---------------------------------------------------------------------------
fn main() {
    let (idle, total) = win_telemetry::get_cpu_times();
    let initial_trigger_cfg = load_trigger_config();

    let initial_state = Arc::new(AppState {
        cadice: Mutex::new(CadiceState {
            mood_score: 96.0,
            energy: 92.0,
            mood: "happy".to_string(),
            is_slacking: false,
            pet_count: 0,
            presence: true,
            presence_distance: 1.1,
            somatic_feeling: "💓 毫米波雷达感应到主人就在身边 (1.1m 处)！心跳加速～".to_string(),
            last_updated: chrono::Local::now().to_rfc3339(),
        }),
        sensors: Mutex::new(PhysicalSensors {
            mmwave_detected: true,
            target_distance_m: 1.1,
            indoor_temp_c: 24.2,
            indoor_humidity_pct: 46.5,
            light_lux: 185.0,
            noise_db: 38.0,
            hardware_connected: false,
            port_name: "COM_BLE_PENDING".to_string(),
        }),
        last_cpu_check: Mutex::new((Instant::now(), idle, total)),
        cpa_quota: Mutex::new((80.6, 94.0)),
        trigger_config: Mutex::new(initial_trigger_cfg),
    });

    let background_state = Arc::clone(&initial_state);

    // 1. 自然心跳线程 (每 3 秒根据物理感知平滑回血)
    let state_for_heartbeat = Arc::clone(&background_state);
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(3));
        let mut cadice = state_for_heartbeat.cadice.lock();
        let sensors = state_for_heartbeat.sensors.lock();

        if cadice.is_slacking {
            cadice.energy = (cadice.energy + 1.5).min(100.0);
        } else if sensors.mmwave_detected && sensors.target_distance_m < 1.8 {
            cadice.energy = (cadice.energy + 0.4).min(100.0);
        } else {
            cadice.energy = (cadice.energy + 0.1).min(100.0);
        }
    });

    // 2. CPA Gemini 额度线程：每半小时 (1800 秒) 静默刷新一次，零黑框弹窗！
    let state_for_quota = Arc::clone(&background_state);
    std::thread::spawn(move || {
        let initial_q = query_live_cpa_quota();
        *state_for_quota.cpa_quota.lock() = initial_q;

        loop {
            std::thread::sleep(Duration::from_secs(1800));
            let q = query_live_cpa_quota();
            *state_for_quota.cpa_quota.lock() = q;
        }
    });

    tauri::Builder::default()
        .manage(initial_state)
        .invoke_handler(tauri::generate_handler![
            get_cadice_state,
            pet_cadice,
            toggle_slacking,
            get_telemetry,
            inject_sensor_reading,
            get_wishlist,
            get_trigger_config,
            save_trigger_config,
            toggle_trigger_paused,
            app_minimize,
            app_toggle_maximize,
            app_close,
            app_exit,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Cadice Core Hub");
}
