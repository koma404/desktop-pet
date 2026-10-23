//! 设置：保存在 exe 旁边的 tuanzi.ini（UTF-8 带 BOM、CRLF 换行，Win7 记事本可以直接编辑）。

use std::path::PathBuf;
use std::time::SystemTime;

pub const FILE_NAME: &str = "tuanzi.ini";

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// 伪装成的文件夹名字
    pub folder_name: String,
    /// 窗口位置（像素），None 表示用默认位置
    pub pos: Option<(i32, i32)>,
    /// 达到这个百分比时团子开始紧张、现出原形
    pub warn: u32,
    /// 达到这个百分比时进入告急状态
    pub alert: u32,
    /// 告急时自动轻度清理
    pub auto_clean: bool,
    /// 两次自动清理的最短间隔（分钟）
    pub cooldown_min: u32,
    /// 告急时跳到最前面
    pub popup_on_alert: bool,
    /// 开机自动启动
    pub autostart: bool,
    /// 检查内存的间隔（秒）
    pub interval_sec: u32,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            folder_name: "资料".into(),
            pos: None,
            warn: 75,
            alert: 85,
            auto_clean: true,
            cooldown_min: 5,
            popup_on_alert: true,
            autostart: true,
            interval_sec: 2,
        }
    }
}

pub fn path() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(FILE_NAME))).unwrap_or_else(|| PathBuf::from(FILE_NAME))
}

/// 设置文件的修改时间，用来发现用户在记事本里改过设置。
pub fn mtime() -> Option<SystemTime> {
    std::fs::metadata(path()).and_then(|m| m.modified()).ok()
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" | "开" | "是" => Some(true),
        "0" | "false" | "no" | "off" | "关" | "否" => Some(false),
        _ => None,
    }
}

impl Config {
    /// 读取设置；第二个返回值表示设置文件原本是否存在。
    pub fn load() -> (Config, bool) {
        match std::fs::read(path()) {
            Ok(bytes) => (Config::parse(&String::from_utf8_lossy(&bytes)), true),
            Err(_) => (Config::default(), false),
        }
    }

    pub fn parse(text: &str) -> Config {
        let mut c = Config::default();
        let (mut px, mut py) = (None, None);
        for line in text.trim_start_matches('\u{feff}').lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') || line.starts_with('[') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
            let num = v.parse::<i64>().ok();
            match k.as_str() {
                "folder_name" => c.folder_name = v.to_string(),
                "warn_percent" => c.warn = num.unwrap_or(c.warn as i64).clamp(0, 100) as u32,
                "alert_percent" => c.alert = num.unwrap_or(c.alert as i64).clamp(0, 100) as u32,
                "auto_clean" => c.auto_clean = parse_bool(v).unwrap_or(c.auto_clean),
                "auto_clean_interval_min" => c.cooldown_min = num.unwrap_or(c.cooldown_min as i64).clamp(0, 10_000) as u32,
                "popup_on_alert" => c.popup_on_alert = parse_bool(v).unwrap_or(c.popup_on_alert),
                "autostart" => c.autostart = parse_bool(v).unwrap_or(c.autostart),
                "check_interval_sec" => c.interval_sec = num.unwrap_or(c.interval_sec as i64).clamp(0, 10_000) as u32,
                "pos_x" => px = num.map(|n| n.clamp(-100_000, 100_000) as i32),
                "pos_y" => py = num.map(|n| n.clamp(-100_000, 100_000) as i32),
                _ => {}
            }
        }
        if let (Some(x), Some(y)) = (px, py) {
            c.pos = Some((x, y));
        }
        c.sanitize();
        c
    }

    /// 把不合理的值拉回合理范围。
    pub fn sanitize(&mut self) {
        let name: String = self.folder_name.chars().filter(|c| !c.is_control()).take(40).collect();
        let name = name.trim();
        self.folder_name = if name.is_empty() { Config::default().folder_name } else { name.to_string() };
        self.warn = self.warn.clamp(30, 97);
        if self.alert <= self.warn {
            self.alert = self.warn + 5;
        }
        self.alert = self.alert.min(99);
        self.cooldown_min = self.cooldown_min.clamp(1, 240);
        self.interval_sec = self.interval_sec.clamp(1, 60);
    }

    pub fn to_text(&self) -> String {
        let b = |v: bool| if v { 1 } else { 0 };
        let mut lines = vec![
            "; 团子桌宠的设置文件。修改并保存后，团子会在几秒内自动读取。".to_string(),
            "; 以分号开头的行是说明文字。开关项：1 = 开启，0 = 关闭。".to_string(),
            String::new(),
            "[团子]".to_string(),
            "; 伪装成的文件夹名字".to_string(),
            format!("folder_name={}", self.folder_name),
            "; 内存占用达到多少（%）时，团子开始紧张并现出原形".to_string(),
            format!("warn_percent={}", self.warn),
            "; 内存占用达到多少（%）时进入告急状态".to_string(),
            format!("alert_percent={}", self.alert),
            "; 告急时自动轻度清理".to_string(),
            format!("auto_clean={}", b(self.auto_clean)),
            "; 两次自动清理之间至少间隔几分钟".to_string(),
            format!("auto_clean_interval_min={}", self.cooldown_min),
            "; 告急时跳到所有窗口最前面".to_string(),
            format!("popup_on_alert={}", b(self.popup_on_alert)),
            "; 开机自动启动".to_string(),
            format!("autostart={}", b(self.autostart)),
            "; 每隔几秒检查一次内存".to_string(),
            format!("check_interval_sec={}", self.interval_sec),
        ];
        if let Some((x, y)) = self.pos {
            lines.push("; 团子在屏幕上的位置（拖动后自动记录，删掉这两行就会回到默认位置）".to_string());
            lines.push(format!("pos_x={}", x));
            lines.push(format!("pos_y={}", y));
        }
        let mut s = String::from("\u{feff}");
        for l in lines {
            s.push_str(&l);
            s.push_str("\r\n");
        }
        s
    }

    /// 保存设置，成功时返回保存后文件的修改时间。
    pub fn save(&self) -> Option<SystemTime> {
        std::fs::write(path(), self.to_text()).ok()?;
        mtime()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut c = Config::default();
        c.folder_name = "我的 资料".into();
        c.pos = Some((-12, 340));
        c.warn = 70;
        c.alert = 80;
        c.auto_clean = false;
        assert_eq!(Config::parse(&c.to_text()), c);
    }

    #[test]
    fn sanitizes_bad_values() {
        let c = Config::parse("warn_percent=95\nalert_percent=10\nfolder_name=   \ncheck_interval_sec=0\nauto_clean=关");
        assert_eq!(c.warn, 95);
        assert_eq!(c.alert, 99);
        assert_eq!(c.folder_name, "资料");
        assert_eq!(c.interval_sec, 1);
        assert!(!c.auto_clean);
    }

    #[test]
    fn ignores_half_position() {
        assert_eq!(Config::parse("pos_x=5").pos, None);
    }
}
