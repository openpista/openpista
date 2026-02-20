import re

def fix_file(filepath, module_prefix):
    with open(filepath, "r") as f:
        content = f.read()
    
    # We are looking for something that calls WorkerReport::new(..., 0, "hi\n", "", "stdout:\nhi\n\nexit_code: 0");
    pattern = r'0,\s*"hi\\n",\s*"",\s*"stdout:\\nhi\\n\\nexit_code: 0",'
    replacement = f"""{module_prefix}WorkerOutput {{
                exit_code: 0,
                stdout: "hi\\n".into(),
                stderr: "".into(),
                output: "stdout:\\nhi\\n\\nexit_code: 0".into(),
            }},"""
            
    content = re.sub(pattern, replacement, content)
    with open(filepath, "w") as f:
        f.write(content)

fix_file("crates/agent/src/runtime.rs", "proto::")
fix_file("crates/cli/src/main.rs", "")
