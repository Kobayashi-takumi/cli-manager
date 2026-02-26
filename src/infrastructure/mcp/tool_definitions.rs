use serde_json::{json, Value};

pub fn get_tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "terminal_list",
            "description": "List all terminal windows managed by CLI Manager",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
        json!({
            "name": "terminal_create",
            "description": "Create a new terminal window",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name for the new terminal (auto-generated if omitted)"
                    },
                    "command": {
                        "type": "string",
                        "description": "Command to run in the new terminal (default: $SHELL)"
                    }
                },
                "required": []
            }
        }),
        json!({
            "name": "terminal_kill",
            "description": "Kill (close) a terminal window",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": {
                        "type": "integer",
                        "description": "Terminal ID to kill"
                    }
                },
                "required": ["target"]
            }
        }),
        json!({
            "name": "terminal_select",
            "description": "Select (activate) a terminal window in the TUI",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": {
                        "type": "integer",
                        "description": "Terminal ID to select"
                    }
                },
                "required": ["target"]
            }
        }),
        json!({
            "name": "terminal_rename",
            "description": "Rename a terminal window",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": {
                        "type": "integer",
                        "description": "Terminal ID to rename"
                    },
                    "name": {
                        "type": "string",
                        "description": "New name for the terminal"
                    }
                },
                "required": ["target", "name"]
            }
        }),
        json!({
            "name": "terminal_send_keys",
            "description": "Send keystrokes to a terminal. Supports special keys: Enter, Tab, Escape, Space, BSpace, C-a through C-z",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": {
                        "type": "integer",
                        "description": "Terminal ID to send keys to"
                    },
                    "keys": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Array of keys/text to send"
                    }
                },
                "required": ["target", "keys"]
            }
        }),
        json!({
            "name": "terminal_capture",
            "description": "Capture the current visible output of a terminal",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": {
                        "type": "integer",
                        "description": "Terminal ID to capture"
                    },
                    "include_scrollback": {
                        "type": "boolean",
                        "description": "Include scrollback buffer content (default: false)"
                    }
                },
                "required": ["target"]
            }
        }),
        json!({
            "name": "buffer_get",
            "description": "Get the current yank buffer content",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
        json!({
            "name": "buffer_set",
            "description": "Set the yank buffer content",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "Text to set in the yank buffer"
                    }
                },
                "required": ["text"]
            }
        }),
        json!({
            "name": "buffer_paste",
            "description": "Paste the yank buffer content into a terminal",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": {
                        "type": "integer",
                        "description": "Terminal ID to paste into"
                    }
                },
                "required": ["target"]
            }
        }),
        json!({
            "name": "notify",
            "description": "Send a desktop notification through CLI Manager",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Notification title (default: 'CLI Manager')"
                    },
                    "body": {
                        "type": "string",
                        "description": "Notification body text"
                    }
                },
                "required": ["body"]
            }
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Tests: tool count
    // ========================================================================

    #[test]
    fn tool_definitions_returns_11_tools() {
        let tools = get_tool_definitions();
        assert_eq!(tools.len(), 11);
    }

    // ========================================================================
    // Tests: structural validity
    // ========================================================================

    #[test]
    fn all_tools_have_name_and_description() {
        for tool in get_tool_definitions() {
            assert!(
                tool.get("name").is_some(),
                "tool missing name: {:?}",
                tool
            );
            assert!(
                tool.get("description").is_some(),
                "tool missing description: {:?}",
                tool
            );
            assert!(
                tool.get("inputSchema").is_some(),
                "tool missing inputSchema: {:?}",
                tool
            );
        }
    }

    #[test]
    fn all_tools_have_string_name_and_description() {
        for tool in get_tool_definitions() {
            assert!(
                tool["name"].is_string(),
                "tool name should be string: {:?}",
                tool
            );
            assert!(
                tool["description"].is_string(),
                "tool description should be string: {:?}",
                tool
            );
        }
    }

    #[test]
    fn all_input_schemas_have_type_object() {
        for tool in get_tool_definitions() {
            let schema = &tool["inputSchema"];
            assert_eq!(
                schema["type"], "object",
                "inputSchema type should be 'object' for tool {}",
                tool["name"]
            );
        }
    }

    #[test]
    fn all_input_schemas_have_properties_and_required() {
        for tool in get_tool_definitions() {
            let schema = &tool["inputSchema"];
            assert!(
                schema.get("properties").is_some(),
                "inputSchema missing properties for tool {}",
                tool["name"]
            );
            assert!(
                schema.get("required").is_some(),
                "inputSchema missing required for tool {}",
                tool["name"]
            );
        }
    }

    // ========================================================================
    // Tests: tool names
    // ========================================================================

    #[test]
    fn tool_names_are_correct() {
        let tools = get_tool_definitions();
        let names: Vec<&str> = tools
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"terminal_list"));
        assert!(names.contains(&"terminal_create"));
        assert!(names.contains(&"terminal_kill"));
        assert!(names.contains(&"terminal_select"));
        assert!(names.contains(&"terminal_rename"));
        assert!(names.contains(&"terminal_send_keys"));
        assert!(names.contains(&"terminal_capture"));
        assert!(names.contains(&"buffer_get"));
        assert!(names.contains(&"buffer_set"));
        assert!(names.contains(&"buffer_paste"));
        assert!(names.contains(&"notify"));
    }

    #[test]
    fn tool_names_are_unique() {
        let tools = get_tool_definitions();
        let names: Vec<&str> = tools
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(names.len(), unique.len(), "tool names should be unique");
    }

    // ========================================================================
    // Tests: required fields per tool
    // ========================================================================

    fn find_tool(name: &str) -> Value {
        get_tool_definitions()
            .into_iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("tool not found: {name}"))
    }

    #[test]
    fn terminal_list_has_no_required_fields() {
        let tool = find_tool("terminal_list");
        let required = tool["inputSchema"]["required"].as_array().unwrap();
        assert!(required.is_empty());
    }

    #[test]
    fn terminal_create_has_no_required_fields() {
        let tool = find_tool("terminal_create");
        let required = tool["inputSchema"]["required"].as_array().unwrap();
        assert!(required.is_empty());
    }

    #[test]
    fn terminal_kill_requires_target() {
        let tool = find_tool("terminal_kill");
        let required = tool["inputSchema"]["required"].as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0], "target");
    }

    #[test]
    fn terminal_select_requires_target() {
        let tool = find_tool("terminal_select");
        let required = tool["inputSchema"]["required"].as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0], "target");
    }

    #[test]
    fn terminal_rename_requires_target_and_name() {
        let tool = find_tool("terminal_rename");
        let required = tool["inputSchema"]["required"].as_array().unwrap();
        assert_eq!(required.len(), 2);
        let req_strs: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(req_strs.contains(&"target"));
        assert!(req_strs.contains(&"name"));
    }

    #[test]
    fn terminal_send_keys_requires_target_and_keys() {
        let tool = find_tool("terminal_send_keys");
        let required = tool["inputSchema"]["required"].as_array().unwrap();
        assert_eq!(required.len(), 2);
        let req_strs: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(req_strs.contains(&"target"));
        assert!(req_strs.contains(&"keys"));
    }

    #[test]
    fn terminal_capture_requires_target() {
        let tool = find_tool("terminal_capture");
        let required = tool["inputSchema"]["required"].as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0], "target");
    }

    #[test]
    fn buffer_get_has_no_required_fields() {
        let tool = find_tool("buffer_get");
        let required = tool["inputSchema"]["required"].as_array().unwrap();
        assert!(required.is_empty());
    }

    #[test]
    fn buffer_set_requires_text() {
        let tool = find_tool("buffer_set");
        let required = tool["inputSchema"]["required"].as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0], "text");
    }

    #[test]
    fn buffer_paste_requires_target() {
        let tool = find_tool("buffer_paste");
        let required = tool["inputSchema"]["required"].as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0], "target");
    }

    #[test]
    fn notify_requires_body() {
        let tool = find_tool("notify");
        let required = tool["inputSchema"]["required"].as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0], "body");
    }

    #[test]
    fn notify_has_title_and_body_properties() {
        let tool = find_tool("notify");
        let props = &tool["inputSchema"]["properties"];
        assert_eq!(props["title"]["type"], "string");
        assert_eq!(props["body"]["type"], "string");
    }

    // ========================================================================
    // Tests: property types
    // ========================================================================

    #[test]
    fn terminal_create_has_optional_name_and_command_properties() {
        let tool = find_tool("terminal_create");
        let props = &tool["inputSchema"]["properties"];
        assert_eq!(props["name"]["type"], "string");
        assert_eq!(props["command"]["type"], "string");
    }

    #[test]
    fn terminal_send_keys_has_keys_array_property() {
        let tool = find_tool("terminal_send_keys");
        let props = &tool["inputSchema"]["properties"];
        assert_eq!(props["keys"]["type"], "array");
        assert_eq!(props["keys"]["items"]["type"], "string");
    }

    #[test]
    fn terminal_capture_has_include_scrollback_boolean_property() {
        let tool = find_tool("terminal_capture");
        let props = &tool["inputSchema"]["properties"];
        assert_eq!(props["include_scrollback"]["type"], "boolean");
    }

    #[test]
    fn target_properties_are_integer_type() {
        for name in &[
            "terminal_kill",
            "terminal_select",
            "terminal_rename",
            "terminal_send_keys",
            "terminal_capture",
            "buffer_paste",
        ] {
            let tool = find_tool(name);
            let props = &tool["inputSchema"]["properties"];
            assert_eq!(
                props["target"]["type"], "integer",
                "target should be integer for tool {name}"
            );
        }
    }
}
