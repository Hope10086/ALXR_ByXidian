#![allow(non_upper_case_globals, non_snake_case, clippy::missing_safety_doc)]

mod connection;
mod connection_utils;

use std::{ffi::CStr, ffi::CString, str::FromStr};

// mod logging_backend;

// #[cfg(target_os = "android")]
// mod audio;

include!(concat!(env!("OUT_DIR"), "/oxr_bindings.rs"));

use alvr_common::{prelude::*, ALVR_NAME, ALVR_VERSION};
use alvr_sockets::{
    HeadsetInfoPacket,
    PrivateIdentity,
    //sockets::{LOCAL_IP}
};
// // use jni::{
// //     objects::{JClass, JObject, JString},
// //     JNIEnv,
// // };
use lazy_static::lazy_static;
use parking_lot::Mutex;
use std::{
    ptr, slice,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::{runtime::Runtime, sync::mpsc, sync::Notify};

use local_ipaddress;

//#[cfg(not(target_os = "android"))]
use structopt::{clap::arg_enum, StructOpt};

lazy_static! {
    pub static ref MAYBE_RUNTIME: Mutex<Option<Runtime>> = Mutex::new(None);
    static ref IDR_REQUEST_NOTIFIER: Notify = Notify::new();
    static ref IDR_PARSED: AtomicBool = AtomicBool::new(false);
    static ref MAYBE_LEGACY_SENDER: Mutex<Option<mpsc::UnboundedSender<Vec<u8>>>> =
        Mutex::new(None);
    pub static ref ON_PAUSE_NOTIFIER: Notify = Notify::new();
}

#[cfg(not(target_os = "android"))]
lazy_static! {
    pub static ref APP_CONFIG: Options = Options::from_args();
}

#[cfg(target_os = "android")]
pub const APP_CONFIG: Options = Options {
    localhost: false,
    graphics_api: Some(crate::GraphicsCtxApi::Auto),
};

pub extern "C" fn init_connections(sysProp: *const crate::SystemProperties) {
    //println!("Hello world\n");
    alvr_common::show_err(|| -> StrResult {
        println!("Hello world\n");

        // // struct OnResumeResult {
        // //     DeviceType deviceType;
        // //     int recommendedEyeWidth;
        // //     int recommendedEyeHeight;
        // //     float *refreshRates;
        // //     int refreshRatesCount;
        // // };

        // let java_vm = trace_err!(env.get_java_vm())?;
        // let activity_ref = trace_err!(env.new_global_ref(jactivity))?;
        // let nal_class_ref = trace_err!(env.new_global_ref(nal_class))?;

        //let result = onResumeNative(*jscreen_surface as _, dark_mode == 1);

        // let device_name = if result.deviceType == DeviceType_OCULUS_GO {
        //     "Oculus Go"
        // } else if result.deviceType == DeviceType_OCULUS_QUEST {
        //     "Oculus Quest"
        // } else if result.deviceType == DeviceType_OCULUS_QUEST_2 {
        //     "Oculus Quest 2"
        // } else {
        //     "Unknown device"
        // };

        let systemProperties = unsafe { *sysProp };
        let system_name = unsafe { CStr::from_ptr(systemProperties.systemName.as_ptr()) };
        let device_name: &str = system_name.to_str().unwrap_or("UnknownHMD");
        //let device_name = unsafe { CStr::from_ptr((*sysProp).systemName.as_ptr()).to_string_lossy().into_owned() };

        //let current_refresh_rate = systemProperties.currentRefreshRate;
        let available_refresh_rates = unsafe {
            slice::from_raw_parts(
                systemProperties.refreshRates,
                systemProperties.refreshRatesCount as _,
            )
            .to_vec()
        }; //vec![90.0];
        let preferred_refresh_rate = available_refresh_rates.last().cloned().unwrap_or(60_f32); //90.0;

        let headset_info = HeadsetInfoPacket {
            recommended_eye_width: systemProperties.recommendedEyeWidth as _,
            recommended_eye_height: systemProperties.recommendedEyeHeight as _,
            available_refresh_rates,
            preferred_refresh_rate,
            reserved: format!("{}", *ALVR_VERSION),
        };

        println!(
            "recommended eye width: {0}, height: {1}",
            headset_info.recommended_eye_width, headset_info.recommended_eye_height
        );

        let ipAddr = if APP_CONFIG.localhost {
            std::net::Ipv4Addr::LOCALHOST.to_string()
        } else {
            local_ipaddress::get().unwrap_or(alvr_sockets::LOCAL_IP.to_string())
        };
        let private_identity = alvr_sockets::create_identity(Some(ipAddr)).unwrap(); /*PrivateIdentity {
                                                                                         hostname: //trace_err!(env.get_string(jhostname))?.into(),
                                                                                         certificate_pem: //trace_err!(env.get_string(jcertificate_pem))?.into(),
                                                                                         key_pem: //trace_err!(env.get_string(jprivate_key))?.into(),
                                                                                     };*/

        let runtime = trace_err!(Runtime::new())?;

        runtime.spawn(async move {
            let connection_loop = connection::connection_lifecycle_loop(
                headset_info,
                device_name,
                private_identity,
                // Arc::new(java_vm),
                // Arc::new(activity_ref),
                // Arc::new(nal_class_ref),
            );

            tokio::select! {
                _ = connection_loop => (),
                _ = ON_PAUSE_NOTIFIER.notified() => ()
            };
        });

        *MAYBE_RUNTIME.lock() = Some(runtime);

        Ok(())
    }());
}

pub fn shutdown() {
    ON_PAUSE_NOTIFIER.notify_waiters();
    drop(MAYBE_RUNTIME.lock().take());
}

pub extern "C" fn legacy_send(
    buffer_ptr: *const ::std::os::raw::c_uchar,
    len: ::std::os::raw::c_uint,
) {
    if let Some(sender) = &*MAYBE_LEGACY_SENDER.lock() {
        let mut vec_buffer = vec![0; len as _];

        // use copy_nonoverlapping (aka memcpy) to avoid freeing memory allocated by C++
        unsafe {
            ptr::copy_nonoverlapping(buffer_ptr, vec_buffer.as_mut_ptr(), len as _);
        }

        sender.send(vec_buffer).ok();
    }
}

impl From<&str> for crate::GraphicsCtxApi {
    fn from(input: &str) -> Self {
        let trimmed = input.trim();
        match trimmed {
            "Vulkan2" => crate::GraphicsCtxApi::Vulkan2,
            "Vulkan" => crate::GraphicsCtxApi::Vulkan,
            "D3D12" => crate::GraphicsCtxApi::D3D12,
            "D3D11" => crate::GraphicsCtxApi::D3D11,
            "OpenGLES" => crate::GraphicsCtxApi::OpenGLES,
            "OpenGL" => crate::GraphicsCtxApi::OpenGL,
            _ => crate::GraphicsCtxApi::Auto,
        }
    }
}

#[derive(Debug, StructOpt)]
#[structopt(name = "openxr_client", about = "An OpenXR based ALVR client.")]
pub struct Options {
    /// Activate debug mode
    // short and long flags (-d, --debug) will be deduced from the field's name
    #[structopt(/*short,*/ long)]
    pub localhost: bool,

    #[structopt(short = "g", long = "graphics", parse(from_str))]
    pub graphics_api: Option<crate::GraphicsCtxApi>,
    // /// Set speed
    // // we don't want to name it "speed", need to look smart
    // #[structopt(short = "v", long = "velocity", default_value = "42")]
    // speed: f64,

    // /// Input file
    // #[structopt(parse(from_os_str))]
    // input: PathBuf,

    // /// Output file, stdout if not present
    // #[structopt(parse(from_os_str))]
    // output: Option<PathBuf>,

    // /// Where to write the output: to `stdout` or `file`
    // #[structopt(short)]
    // out_type: String,

    // /// File name: only required when `out-type` is set to `file`
    // #[structopt(name = "FILE", required_if("out-type", "file"))]
    // file_name: Option<String>,
}
