import re
import sys

def replace_in_proto():
    with open("crates/proto/src/event.rs", "r") as f:
        content = f.read()
    
    # define the struct
    new_struct = """
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerOutput {
    pub exit_code: i64,
    pub stdout: String,
    pub stderr: String,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerReport {
"""
    content = content.replace("#[derive(Debug, Clone, Serialize, Deserialize)]\npub struct WorkerReport {", new_struct)
    
    # replace new signature
    old_sig = """    pub fn new(
        call_id: impl Into<String>,
        worker_id: impl Into<String>,
        image: impl Into<String>,
        command: impl Into<String>,
        exit_code: i64,
        stdout: impl Into<String>,
        stderr: impl Into<String>,
        output: impl Into<String>,
    ) -> Self {"""
    new_sig = """    pub fn new(
        call_id: impl Into<String>,
        worker_id: impl Into<String>,
        image: impl Into<String>,
        command: impl Into<String>,
        worker_output: WorkerOutput,
    ) -> Self {"""
    content = content.replace(old_sig, new_sig)
    
    # replace body
    old_body = """            exit_code,
            stdout: stdout.into(),
            stderr: stderr.into(),
            output: output.into(),
        }"""
    new_body = """            exit_code: worker_output.exit_code,
            stdout: worker_output.stdout,
            stderr: worker_output.stderr,
            output: worker_output.output,
        }"""
    content = content.replace(old_body, new_body)
    
    # replace test
    old_test = """        let report = WorkerReport::new(
            "call-1",
            "worker-a",
            "alpine:3.20",
            "echo hi",
            0,
            "hi\\n",
            "",
            "stdout:\\nhi\\n\\nexit_code: 0",
        );"""
    new_test = """        let report = WorkerReport::new(
            "call-1",
            "worker-a",
            "alpine:3.20",
            "echo hi",
            WorkerOutput {
                exit_code: 0,
                stdout: "hi\\n".into(),
                stderr: "".into(),
                output: "stdout:\\nhi\\n\\nexit_code: 0".into(),
            },
        );"""
    content = content.replace(old_test, new_test)
    
    with open("crates/proto/src/event.rs", "w") as f:
        f.write(content)

def replace_in_agent():
    with open("crates/agent/src/runtime.rs", "r") as f:
        content = f.read()
    old_test = """        let report = proto::WorkerReport::new(
            "call-xyz",
            "worker-123",
            "alpine",
            "echo test",
            0,
            "test_stdout",
            "test_stderr",
            "test_output",
        );"""
    new_test = """        let report = proto::WorkerReport::new(
            "call-xyz",
            "worker-123",
            "alpine",
            "echo test",
            proto::WorkerOutput {
                exit_code: 0,
                stdout: "test_stdout".into(),
                stderr: "test_stderr".into(),
                output: "test_output".into(),
            },
        );"""
    content = content.replace(old_test, new_test)
    with open("crates/agent/src/runtime.rs", "w") as f:
        f.write(content)

def replace_in_cli():
    with open("crates/cli/src/main.rs", "r") as f:
        content = f.read()
    old_test = """        let report = WorkerReport::new(
            "call_abc",
            "test_worker",
            "ubuntu",
            "ls -la",
            0,
            "stdout_text",
            "stderr_text",
            "full_output",
        );"""
    new_test = """        let report = WorkerReport::new(
            "call_abc",
            "test_worker",
            "ubuntu",
            "ls -la",
            proto::WorkerOutput {
                exit_code: 0,
                stdout: "stdout_text".into(),
                stderr: "stderr_text".into(),
                output: "full_output".into(),
            },
        );"""
    content = content.replace(old_test, new_test)
    with open("crates/cli/src/main.rs", "w") as f:
        f.write(content)

def replace_in_tools():
    with open("crates/tools/src/container.rs", "r") as f:
        content = f.read()
    old_call = """    let report = WorkerReport::new(
        call_id.to_string(),
        container_name.to_string(),
        args.image.clone(),
        args.command.clone(),
        execution.exit_code,
        execution.stdout.clone(),
        execution.stderr.clone(),
        output,
    );"""
    new_call = """    let report = WorkerReport::new(
        call_id.to_string(),
        container_name.to_string(),
        args.image.clone(),
        args.command.clone(),
        proto::WorkerOutput {
            exit_code: execution.exit_code,
            stdout: execution.stdout.clone(),
            stderr: execution.stderr.clone(),
            output,
        },
    );"""
    content = content.replace(old_call, new_call)
    with open("crates/tools/src/container.rs", "w") as f:
        f.write(content)

replace_in_proto()
replace_in_agent()
replace_in_cli()
replace_in_tools()
