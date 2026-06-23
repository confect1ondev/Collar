use collar_common::{DaemonMessage, ScriptInfo, ScriptType};

fn main() {
    let scripts = vec![
        ScriptInfo {
            id: "lock".to_string(),
            name: "Lock".to_string(),
            description: "Lock screen".to_string(),
            script_type: ScriptType::Action,
            icon: None,
        }
    ];
    
    let msg = DaemonMessage::Auth {
        device_key: "test-key".to_string(),
        scripts,
        lan_ip: Some("192.168.5.215".to_string()),
    };
    
    println!("{}", serde_json::to_string_pretty(&msg).unwrap());
}
