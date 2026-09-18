use crate::Result;
use jni::{
    objects::{GlobalRef, JClass, JString, JValue},
    JNIEnv, JavaVM,
};
use std::sync::OnceLock;

struct Bridge {
    vm: JavaVM,
    class: GlobalRef,
}
static BRIDGE: OnceLock<Bridge> = OnceLock::new();

#[no_mangle]
pub extern "system" fn Java_org_agent_1runtime_mobile_NativeBridge_nativeInit(
    env: JNIEnv,
    class: JClass,
) {
    if let (Ok(vm), Ok(class)) = (env.get_java_vm(), env.new_global_ref(class)) {
        let _ = BRIDGE.set(Bridge { vm, class });
    }
}
fn with<T>(f: impl FnOnce(&mut JNIEnv, &JClass) -> jni::errors::Result<T>) -> Result<T> {
    let bridge = BRIDGE.get().ok_or("android_bridge_unavailable")?;
    let mut env = bridge
        .vm
        .attach_current_thread()
        .map_err(|_| "android_bridge_unavailable")?;
    let class: &JClass = bridge.class.as_obj().into();
    match f(&mut env, class) {
        Ok(value) => Ok(value),
        Err(_) => {
            // Clear without describing the Java exception: messages can contain secrets.
            let _ = env.exception_clear();
            Err("android_operation_failed".into())
        }
    }
}
pub fn string(method: &str, values: &[&str]) -> Result<Option<String>> {
    with(|env, class| {
        let objects: Vec<JString> = values
            .iter()
            .map(|v| env.new_string(v))
            .collect::<jni::errors::Result<_>>()?;
        let args: Vec<JValue> = objects.iter().map(|v| JValue::Object(v.as_ref())).collect();
        let sig = format!(
            "({})Ljava/lang/String;",
            "Ljava/lang/String;".repeat(values.len())
        );
        let out = env.call_static_method(class, method, sig, &args)?.l()?;
        if out.is_null() {
            Ok(None)
        } else {
            Ok(Some(env.get_string(&JString::from(out))?.into()))
        }
    })
}
pub fn start_worker() -> Result<i32> {
    with(|env, class| {
        env.call_static_method(class, "startWorker", "()I", &[])?
            .i()
    })
}
pub fn stop_worker() -> Result<()> {
    with(|env, class| {
        env.call_static_method(class, "stopWorker", "()V", &[])
            .map(|_| ())
    })
}
pub fn locked() -> Result<bool> {
    with(|env, class| env.call_static_method(class, "isLocked", "()Z", &[])?.z())
}
#[no_mangle]
pub extern "system" fn Java_org_agent_1runtime_mobile_NativeBridge_runWorker(
    _env: JNIEnv,
    _class: JClass,
    fd: i32,
) {
    use std::os::fd::FromRawFd;
    if fd < 0 {
        return;
    }
    // SAFETY: Java detached this descriptor and transfers sole ownership here.
    let input = unsafe { std::fs::File::from_raw_fd(fd) };
    if let Ok(output) = input.try_clone() {
        let _ = crate::wallet::worker::run_io(input, output);
    }
}
