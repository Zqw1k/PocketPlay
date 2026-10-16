//! JNI is confined to Android framework operations; media packets stay native.
use anyhow::{bail, Context, Result};
use jni::{
    objects::{GlobalRef, JClass, JFieldID, JObject, JObjectArray, JString, JValue},
    JNIEnv, JavaVM,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

#[derive(Clone)]
pub(crate) struct Framework {
    pub vm: Arc<JavaVM>,
    context: GlobalRef,
    manager: GlobalRef,
    panel_off: Arc<AtomicBool>,
    display_control: Arc<Mutex<Option<GlobalRef>>>,
    blanked_panel: Arc<Mutex<Option<BlankedPanel>>>,
}

struct BlankedPanel {
    token: GlobalRef,
    brightness: f32,
}

pub(crate) struct SessionPower {
    framework: Framework,
    wake_lock: GlobalRef,
}

impl Drop for SessionPower {
    fn drop(&mut self) {
        if let Ok(mut env) = self.framework.vm.attach_current_thread() {
            let _ = env.exception_clear();
            if let Err(error) = self.framework.panel_power(&mut env, false) {
                eprintln!("Could not restore display: {error:#}");
                let _ = env.exception_clear();
            }
            let _ = env.call_method(self.wake_lock.as_obj(), "release", "()V", &[]);
            let _ = env.exception_clear();
            println!("Session wake lock released");
        }
    }
}

pub(crate) struct DisplayState {
    pub width: i32,
    pub height: i32,
    pub dpi: i32,
    pub mode_id: i32,
}

impl Framework {
    pub fn context(&self) -> &JObject<'static> {
        self.context.as_obj()
    }
    pub fn new(env: &mut JNIEnv) -> Result<Self> {
        // Initialize the event class before taking the first gesture timestamp.
        // JNI static-method lookup initializes the class; doing it on the first
        // DOWN would include cold framework initialization in that event's age.
        env.get_static_method_id(
            "android/view/MotionEvent",
            "obtain",
            "(JJII[Landroid/view/MotionEvent$PointerProperties;[Landroid/view/MotionEvent$PointerCoords;IIFFIIII)Landroid/view/MotionEvent;",
        )?;
        println!("Native framework: prepare looper");
        env.call_static_method("android/os/Looper", "prepareMainLooper", "()V", &[])?;
        println!("Native framework: initialize thread");
        let thread = env
            .call_static_method(
                "android/app/ActivityThread",
                "systemMain",
                "()Landroid/app/ActivityThread;",
                &[],
            )?
            .l()?;
        println!("Native framework: get context");
        let base = env
            .call_method(
                &thread,
                "getSystemContext",
                "()Landroid/app/ContextImpl;",
                &[],
            )?
            .l()?;
        let package = env.new_string("com.android.shell")?;
        let context = env
            .call_method(
                &base,
                "createPackageContext",
                "(Ljava/lang/String;I)Landroid/content/Context;",
                &[JValue::Object(&package), JValue::Int(0)],
            )?
            .l()?;
        let attribution = env
            .call_method(
                &context,
                "getAttributionSource",
                "()Landroid/content/AttributionSource;",
                &[],
            )?
            .l()?;
        println!(
            "Shell attribution UID: {} | process UID: {}",
            env.call_method(&attribution, "getUid", "()I", &[])?.i()?,
            unsafe { libc::getuid() }
        );
        let builder = env.new_object(
            "android/content/AttributionSource$Builder",
            "(I)V",
            &[JValue::Int(unsafe { libc::getuid() } as i32)],
        )?;
        env.call_method(
            &builder,
            "setPackageName",
            "(Ljava/lang/String;)Landroid/content/AttributionSource$Builder;",
            &[JValue::Object(&package)],
        )?;
        let attribution = env
            .call_method(
                builder,
                "build",
                "()Landroid/content/AttributionSource;",
                &[],
            )?
            .l()?;
        env.set_field(
            &context,
            "mAttributionSource",
            "Landroid/content/AttributionSource;",
            JValue::Object(&attribution),
        )?;
        let application = env.new_object("android/app/Application", "()V", &[])?;
        env.call_method(
            &application,
            "attach",
            "(Landroid/content/Context;)V",
            &[JValue::Object(&context)],
        )?;
        env.set_field(
            &thread,
            "mInitialApplication",
            "Landroid/app/Application;",
            JValue::Object(&application),
        )?;
        let info = env
            .call_method(
                &context,
                "getApplicationInfo",
                "()Landroid/content/pm/ApplicationInfo;",
                &[],
            )?
            .l()?;
        let bound = env.new_object("android/app/ActivityThread$AppBindData", "()V", &[])?;
        env.set_field(
            &bound,
            "appInfo",
            "Landroid/content/pm/ApplicationInfo;",
            JValue::Object(&info),
        )?;
        env.set_field(
            &thread,
            "mBoundApplication",
            "Landroid/app/ActivityThread$AppBindData;",
            JValue::Object(&bound),
        )?;
        let manager = env
            .call_static_method(
                "android/hardware/display/DisplayManagerGlobal",
                "getInstance",
                "()Landroid/hardware/display/DisplayManagerGlobal;",
                &[],
            )?
            .l()?;
        Ok(Self {
            vm: Arc::new(env.get_java_vm()?),
            context: env.new_global_ref(context)?,
            manager: env.new_global_ref(manager)?,
            panel_off: Arc::new(AtomicBool::new(false)),
            display_control: Arc::new(Mutex::new(None)),
            blanked_panel: Arc::new(Mutex::new(None)),
        })
    }
    pub fn geometry(&self, env: &mut JNIEnv) -> Result<(i32, i32, i32)> {
        let state = self.display_state(env)?;
        Ok((state.width, state.height, state.dpi))
    }
    pub fn display_state(&self, env: &mut JNIEnv) -> Result<DisplayState> {
        env.with_local_frame(8, |env| -> Result<_> {
            let info = env
                .call_method(
                    self.manager.as_obj(),
                    "getDisplayInfo",
                    "(I)Landroid/view/DisplayInfo;",
                    &[JValue::Int(0)],
                )?
                .l()?;
            let w = env.get_field(&info, "logicalWidth", "I")?.i()?;
            let h = env.get_field(&info, "logicalHeight", "I")?.i()?;
            let dpi = env.get_field(&info, "logicalDensityDpi", "I")?.i()?;
            let mode_id = env.get_field(&info, "modeId", "I")?.i()?;
            if !(1..=8192).contains(&w) || !(1..=8192).contains(&h) {
                bail!("Invalid Android display size");
            }
            Ok(DisplayState {
                width: w,
                height: h,
                dpi,
                mode_id,
            })
        })
    }
    pub fn encoders(&self, env: &mut JNIEnv) -> Result<Vec<String>> {
        env.with_local_frame(128, |env| -> Result<_> {
            let list = env.new_object("android/media/MediaCodecList", "(I)V", &[JValue::Int(0)])?;
            let array = JObjectArray::from(
                env.call_method(
                    list,
                    "getCodecInfos",
                    "()[Landroid/media/MediaCodecInfo;",
                    &[],
                )?
                .l()?,
            );
            let mut result = Vec::new();
            for i in 0..env.get_array_length(&array)? {
                env.with_local_frame(16, |env| -> Result<()> {
                    let info = env.get_object_array_element(&array, i)?;
                    if !env.call_method(&info, "isEncoder", "()Z", &[])?.z()?
                        || !env
                            .call_method(&info, "isHardwareAccelerated", "()Z", &[])?
                            .z()?
                    {
                        return Ok(());
                    }
                    let name = JString::from(
                        env.call_method(&info, "getName", "()Ljava/lang/String;", &[])?
                            .l()?,
                    );
                    let name: String = env.get_string(&name)?.into();
                    let types = JObjectArray::from(
                        env.call_method(info, "getSupportedTypes", "()[Ljava/lang/String;", &[])?
                            .l()?,
                    );
                    for j in 0..env.get_array_length(&types)? {
                        let mime = JString::from(env.get_object_array_element(&types, j)?);
                        let mime: String = env.get_string(&mime)?.into();
                        if mime.starts_with("video/") {
                            result.push(format!("{name} | {mime} | hardware"));
                        }
                    }
                    Ok(())
                })?;
            }
            Ok(result)
        })
    }
    #[cfg(feature = "qa")]
    pub fn encoder_parameters(&self, env: &mut JNIEnv, name: &str) -> Result<()> {
        env.with_local_frame(16, |env| -> Result<()> {
            let name = env.new_string(name)?;
            let codec = env.call_static_method("android/media/MediaCodec", "createByCodecName", "(Ljava/lang/String;)Landroid/media/MediaCodec;", &[JValue::Object(&name)])?.l()?;
            let result = (|| -> Result<()> {
                let list = env.call_method(&codec, "getSupportedVendorParameters", "()Ljava/util/List;", &[])?.l()?;
                let size = env.call_method(&list, "size", "()I", &[])?.i()?;
                if !(0..=512).contains(&size) { bail!("Invalid codec parameter count"); }
                for index in 0..size {
                    env.with_local_frame(8, |env| -> Result<()> {
                        let key = JString::from(env.call_method(&list, "get", "(I)Ljava/lang/Object;", &[JValue::Int(index)])?.l()?);
                        let descriptor = env.call_method(&codec, "getParameterDescriptor", "(Ljava/lang/String;)Landroid/media/MediaCodec$ParameterDescriptor;", &[JValue::Object(&key)])?.l()?;
                        let kind = env.call_method(descriptor, "getType", "()I", &[])?.i()?;
                        let key: String = env.get_string(&key)?.into();
                        println!("QA encoder parameter: {key} | type={kind}");
                        Ok(())
                    })?;
                }
                Ok(())
            })();
            if env.exception_check()? { env.exception_clear()?; }
            let released = env.call_method(codec, "release", "()V", &[]);
            result?;
            released?;
            Ok(())
        })
    }
    pub fn mirror(
        &self,
        env: &mut JNIEnv,
        surface: &JObject,
        w: i32,
        h: i32,
        dpi: i32,
        fps: i32,
    ) -> Result<GlobalRef> {
        env.with_local_frame(16, |env| -> Result<_> {
            let name = env.new_string("PocketPlay Native")?;
            let builder = env.new_object("android/hardware/display/VirtualDisplayConfig$Builder", "(Ljava/lang/String;III)V",
                &[JValue::Object(&name), JValue::Int(w), JValue::Int(h), JValue::Int(dpi)])?;
            let sig = "Landroid/hardware/display/VirtualDisplayConfig$Builder;";
            env.call_method(&builder, "setSurface", format!("(Landroid/view/Surface;){sig}"), &[JValue::Object(surface)])?;
            env.call_method(&builder, "setFlags", format!("(I){sig}"), &[JValue::Int(17)])?;
            env.call_method(&builder, "setDisplayIdToMirror", format!("(I){sig}"), &[JValue::Int(0)])?;
            env.call_method(&builder, "setRequestedRefreshRate", format!("(F){sig}"), &[JValue::Float(fps as f32)])?;
            let config = env.call_method(&builder, "build", "()Landroid/hardware/display/VirtualDisplayConfig;", &[])?.l()?;
            let display = env.call_method(self.manager.as_obj(), "createVirtualDisplay",
                "(Landroid/content/Context;Landroid/media/projection/MediaProjection;Landroid/hardware/display/VirtualDisplayConfig;Landroid/hardware/display/VirtualDisplay$Callback;Ljava/util/concurrent/Executor;)Landroid/hardware/display/VirtualDisplay;",
                &[JValue::Object(self.context.as_obj()), JValue::Object(&JObject::null()), JValue::Object(&config), JValue::Object(&JObject::null()), JValue::Object(&JObject::null())])?.l()?;
            if display.is_null() { bail!("Android denied display capture"); }
            Ok(env.new_global_ref(display)?)
        })
    }
    pub fn keep_awake(&self, env: &mut JNIEnv) -> Result<SessionPower> {
        env.with_local_frame(8, |env| -> Result<_> {
            let name = env.new_string("power")?;
            let power = env
                .call_method(
                    self.context(),
                    "getSystemService",
                    "(Ljava/lang/String;)Ljava/lang/Object;",
                    &[JValue::Object(&name)],
                )?
                .l()?;
            if !env.call_method(&power, "isInteractive", "()Z", &[])?.z()? {
                bail!("Unlock the phone manually before starting the stream");
            }
            let tag = env.new_string("PocketPlay:stream")?;
            // No ACQUIRE_CAUSES_WAKEUP: this only retains an already-awake session.
            // A screen wake lock is needed because this shell service has no Activity window.
            let lock = env
                .call_method(
                    power,
                    "newWakeLock",
                    "(ILjava/lang/String;)Landroid/os/PowerManager$WakeLock;",
                    &[JValue::Int(10), JValue::Object(&tag)],
                )?
                .l()?;
            env.call_method(&lock, "setReferenceCounted", "(Z)V", &[JValue::Bool(0)])?;
            let wake_lock = env.new_global_ref(&lock)?;
            env.call_method(lock, "acquire", "()V", &[])?;
            println!("Session wake lock acquired; Android stays awake without a charger");
            Ok(SessionPower {
                framework: self.clone(),
                wake_lock,
            })
        })
    }
    fn physical_display_token(&self, env: &mut JNIEnv) -> Result<GlobalRef> {
        env.with_local_frame(8, |env| -> Result<_> {
            let mut cached=self.display_control.lock().unwrap_or_else(|p|p.into_inner());
            if cached.is_none() {
                let path=env.new_string(std::env::var("SYSTEMSERVERCLASSPATH").unwrap_or_else(|_|"/system/framework/services.jar".into()))?;
                let application_loader=env.call_static_method("java/lang/ClassLoader","getSystemClassLoader","()Ljava/lang/ClassLoader;",&[])?.l()?;
                // Resolve service classes together from SYSTEMSERVERCLASSPATH.
                // Delegating to the app_process loader can resolve DisplayControl
                // from services.jar alone, hiding Honor's vendor service classes
                // from android_servers native registration and aborting the VM.
                let parent=env.call_method(application_loader,"getParent","()Ljava/lang/ClassLoader;",&[])?.l()?;
                // The system framework requires a shared system-library namespace,
                // unlike the isolated namespace used by an application dex loader.
                let loader=env.call_static_method("com/android/internal/os/ClassLoaderFactory","createClassLoader",
                    "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/ClassLoader;IZLjava/lang/String;)Ljava/lang/ClassLoader;",
                    &[JValue::Object(&path),JValue::Object(&JObject::null()),JValue::Object(&JObject::null()),JValue::Object(&parent),JValue::Int(0),JValue::Bool(1),JValue::Object(&JObject::null())])?.l()?;
                let name=env.new_string("com.android.server.display.DisplayControl")?;
                let class=env.call_method(loader,"loadClass","(Ljava/lang/String;)Ljava/lang/Class;",&[JValue::Object(&name)])?.l()?;
                let runtime=env.call_static_method("java/lang/Runtime","getRuntime","()Ljava/lang/Runtime;",&[])?.l()?;
                let library=env.new_string("android_servers")?;
                env.call_method(runtime,"loadLibrary0","(Ljava/lang/Class;Ljava/lang/String;)V",&[JValue::Object(&class),JValue::Object(&library)])?;
                *cached=Some(env.new_global_ref(class)?);
            }
            let class:&JClass=cached.as_ref().context("Display control is missing")?.as_obj().into();
            let ids = jni::objects::JLongArray::from(env.call_static_method(class, "getPhysicalDisplayIds", "()[J", &[])?.l()?);
            if env.get_array_length(&ids)? == 0 { bail!("No physical display"); }
            let mut id = [0]; env.get_long_array_region(&ids, 0, &mut id)?;
            let token = env.call_static_method(class, "getPhysicalDisplayToken", "(J)Landroid/os/IBinder;", &[JValue::Long(id[0])])?.l()?;
            Ok(env.new_global_ref(token)?)
        })
    }
    fn restore_light(&self, env: &mut JNIEnv) -> Result<()> {
        let mut blanked = self.blanked_panel.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(panel) = blanked.as_ref() {
            let restored = env
                .call_static_method(
                    "android/view/SurfaceControl",
                    "setDisplayBrightness",
                    "(Landroid/os/IBinder;F)Z",
                    &[
                        JValue::Object(panel.token.as_obj()),
                        JValue::Float(panel.brightness),
                    ],
                )?
                .z()?;
            if !restored {
                bail!("Android could not restore the display light");
            }
            *blanked = None;
            println!("Display light restored");
        }
        Ok(())
    }
    pub fn panel_power(&self, env: &mut JNIEnv, off: bool) -> Result<()> {
        if !off {
            self.restore_light(env)?;
            if !self.panel_off.load(Ordering::Acquire) {
                return Ok(());
            }
        }
        env.with_local_frame(8, |env| -> Result<()> {
            let token = self.physical_display_token(env)?;
            if off {
                let brand = JString::from(env.get_static_field("android/os/Build", "MANUFACTURER", "Ljava/lang/String;")?.l()?);
                let brand: String = env.get_string(&brand)?.into();
                if ["xiaomi", "redmi", "poco"].iter().any(|value| brand.eq_ignore_ascii_case(value)) {
                    let mut blanked = self.blanked_panel.lock().unwrap_or_else(|p|p.into_inner());
                    if blanked.is_some() { return Ok(()); }
                    let supported = env.call_static_method("android/view/SurfaceControl", "getDisplayBrightnessSupport",
                        "(Landroid/os/IBinder;)Z", &[JValue::Object(token.as_obj())])?.z()?;
                    if !supported { bail!("This Xiaomi firmware cannot blank the display while keeping refresh active"); }
                    let brightness = env.call_method(self.manager.as_obj(), "getBrightness", "(I)F", &[JValue::Int(0)])?.f()?;
                    if !brightness.is_finite() || !(0.0..=1.0).contains(&brightness) {
                        bail!("Could not read a restorable display brightness");
                    }
                    let applied = env.call_static_method("android/view/SurfaceControl", "setDisplayBrightness",
                        "(Landroid/os/IBinder;F)Z", &[JValue::Object(token.as_obj()), JValue::Float(-1.0)])?.z()?;
                    if !applied { bail!("Android rejected display light-off"); }
                    *blanked = Some(BlankedPanel { token, brightness });
                    println!("Display light off; physical refresh timing remains active");
                    return Ok(());
                }
            }
            env.call_static_method("android/view/SurfaceControl", "setDisplayPowerMode", "(Landroid/os/IBinder;I)V",
                &[JValue::Object(token.as_obj()), JValue::Int(if off {0} else {2})])?;
            println!("Display panel {}", if off {"off"} else {"on"});
            self.panel_off.store(off,Ordering::Release);
            Ok(())
        })
    }
}

struct Finger {
    pointer: u64,
    slot: i32,
    x: f32,
    y: f32,
}
fn validate_motion(action: i32, first_move: bool, last: i64, now: i64) -> bool {
    action != 2 || first_move || now.saturating_sub(last) >= 8
}
pub(crate) struct Input {
    manager: GlobalRef,
    properties: GlobalRef,
    coordinates: GlobalRef,
    props: Vec<GlobalRef>,
    coords: Vec<GlobalRef>,
    fingers: Vec<Finger>,
    down_time: i64,
    validate_next_move: bool,
    last_validation: i64,
    pointer_id: JFieldID,
    x_field: JFieldID,
    y_field: JFieldID,
}
impl Input {
    pub fn new(env: &mut JNIEnv) -> Result<Self> {
        let manager = env
            .call_static_method(
                "android/hardware/input/InputManagerGlobal",
                "getInstance",
                "()Landroid/hardware/input/InputManagerGlobal;",
                &[],
            )?
            .l()?;
        let properties = env.new_object_array(
            10,
            "android/view/MotionEvent$PointerProperties",
            JObject::null(),
        )?;
        let coordinates = env.new_object_array(
            10,
            "android/view/MotionEvent$PointerCoords",
            JObject::null(),
        )?;
        let mut props = Vec::with_capacity(10);
        let mut coords = Vec::with_capacity(10);
        for _ in 0..10 {
            let prop = env.new_object("android/view/MotionEvent$PointerProperties", "()V", &[])?;
            env.set_field(&prop, "toolType", "I", JValue::Int(1))?;
            props.push(env.new_global_ref(prop)?);
            let coord = env.new_object("android/view/MotionEvent$PointerCoords", "()V", &[])?;
            env.set_field(&coord, "pressure", "F", JValue::Float(1.))?;
            env.set_field(&coord, "size", "F", JValue::Float(1.))?;
            coords.push(env.new_global_ref(coord)?);
        }
        Ok(Self {
            manager: env.new_global_ref(manager)?,
            properties: env.new_global_ref(properties)?,
            coordinates: env.new_global_ref(coordinates)?,
            props,
            coords,
            fingers: Vec::with_capacity(10),
            down_time: 0,
            validate_next_move: true,
            last_validation: 0,
            pointer_id: env.get_field_id(
                "android/view/MotionEvent$PointerProperties",
                "id",
                "I",
            )?,
            x_field: env.get_field_id("android/view/MotionEvent$PointerCoords", "x", "F")?,
            y_field: env.get_field_id("android/view/MotionEvent$PointerCoords", "y", "F")?,
        })
    }
    fn now(_env: &mut JNIEnv) -> Result<i64> {
        let mut time = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // Android uptime and input-event timestamps both use CLOCK_MONOTONIC.
        if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(time.tv_sec * 1000 + time.tv_nsec / 1_000_000)
    }
    pub fn touch(
        &mut self,
        env: &mut JNIEnv,
        action: u8,
        pointer: u64,
        x: i32,
        y: i32,
    ) -> Result<()> {
        if action > 2 {
            bail!("Invalid touch action");
        }
        let mut index = self.fingers.iter().position(|f| f.pointer == pointer);
        if action == 0 {
            if index.is_some() {
                return Ok(());
            }
            if self.fingers.len() >= 10 {
                bail!("Too many touch pointers");
            }
            let slot = (0..10)
                .find(|slot| self.fingers.iter().all(|f| f.slot != *slot))
                .context("No pointer slot")?;
            index = Some(self.fingers.len());
            self.fingers.push(Finger {
                pointer,
                slot,
                x: x as f32,
                y: y as f32,
            });
        }
        let Some(index) = index else {
            // A still-held pointer can resume after a transient rejection.
            if action == 2 {
                self.touch(env, 0, pointer, x, y)?;
            }
            return Ok(());
        };
        self.fingers[index].x = x as f32;
        self.fingers[index].y = y as f32;
        let event_action = match action {
            0 => {
                if self.fingers.len() == 1 {
                    0
                } else {
                    5 | (index as i32) << 8
                }
            }
            1 => {
                if self.fingers.len() == 1 {
                    1
                } else {
                    6 | (index as i32) << 8
                }
            }
            _ => 2,
        };
        if !self.inject(env, event_action)? {
            eprintln!("Touch gesture {event_action} rejected by Android; canceling owned pointers and keeping the session alive");
            let resume = if action == 2 {
                self.fingers
                    .iter()
                    .map(|f| (f.pointer, f.x as i32, f.y as i32))
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            self.cancel(env)?;
            for (pointer, x, y) in resume {
                self.touch(env, 0, pointer, x, y)?;
            }
            return Ok(());
        }
        if action == 1 {
            self.fingers.remove(index);
        }
        Ok(())
    }
    fn inject(&mut self, env: &mut JNIEnv, action: i32) -> Result<bool> {
        env.with_local_frame(8, |env| -> Result<bool> {
            let properties: &JObjectArray = self.properties.as_obj().into();
            let coordinates: &JObjectArray = self.coordinates.as_obj().into();
            for (i, f) in self.fingers.iter().enumerate() {
                env.set_field_unchecked(self.props[i].as_obj(), self.pointer_id, JValue::Int(f.slot))?;
                env.set_field_unchecked(self.coords[i].as_obj(), self.x_field, JValue::Float(f.x))?;
                env.set_field_unchecked(self.coords[i].as_obj(), self.y_field, JValue::Float(f.y))?;
                env.set_object_array_element(properties, i as i32, self.props[i].as_obj())?;
                env.set_object_array_element(coordinates, i as i32, self.coords[i].as_obj())?;
            }
            let now = Self::now(env)?;
            // DOWN establishes the gesture using this exact event timestamp.
            // Keep gesture metadata consistent across all pointers and moves.
            if action == 0 {
                self.down_time = now;
            }
            let event = env.call_static_method("android/view/MotionEvent", "obtain",
                "(JJII[Landroid/view/MotionEvent$PointerProperties;[Landroid/view/MotionEvent$PointerCoords;IIFFIIII)Landroid/view/MotionEvent;",
                &[JValue::Long(self.down_time), JValue::Long(now), JValue::Int(action), JValue::Int(self.fingers.len() as i32),
                    JValue::Object(properties), JValue::Object(coordinates), JValue::Int(0), JValue::Int(0), JValue::Float(1.), JValue::Float(1.),
                    JValue::Int(-1), JValue::Int(0), JValue::Int(0x1002), JValue::Int(0)])?.l()?;
            #[cfg(feature = "qa")]
            if std::env::var_os("POCKETPLAY_INPUT_TRACE").is_some() {
                let text = JString::from(env.call_method(&event, "toString", "()Ljava/lang/String;", &[])?.l()?);
                eprintln!("Input trace: {}", String::from(env.get_string(&text)?));
            }
            // Validate edges, the first movement, and at most one movement per
            // 8 ms. Other motion remains asynchronous. WAIT_FOR_RESULT checks
            // dispatch acceptance; it does not wait for application processing.
            let checked = validate_motion(action, self.validate_next_move, self.last_validation, now);
            let mode = if checked { 1 } else { 0 };
            if checked { self.last_validation = now; }
            self.validate_next_move = action != 2;
            let injected = env.call_method(self.manager.as_obj(), "injectInputEvent", "(Landroid/view/InputEvent;I)Z", &[JValue::Object(&event), JValue::Int(mode)]);
            let recycled = env.call_method(&event, "recycle", "()V", &[]);
            let accepted = injected?.z()?; recycled?;
            Ok(accepted)
        })
    }
    pub fn cancel(&mut self, env: &mut JNIEnv) -> Result<()> {
        if self.fingers.is_empty() {
            return Ok(());
        }
        let result = self.inject(env, 3);
        self.fingers.clear();
        result.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::validate_motion;
    #[test]
    fn first_movement_and_gesture_edges_are_validated() {
        assert!(validate_motion(2, true, 100, 101));
        assert!(validate_motion(0, false, 100, 101));
        assert!(validate_motion(1, false, 100, 101));
    }
    #[test]
    fn continuous_motion_stays_async_between_periodic_checks() {
        assert!(!validate_motion(2, false, 100, 107));
        assert!(validate_motion(2, false, 100, 108));
    }
}
