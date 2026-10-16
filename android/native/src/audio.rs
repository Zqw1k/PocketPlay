//! Playback capture uses Android framework permission checks; transport is native PCM.
use crate::{framework::Framework, MAGIC};
use anyhow::{bail, Result};
use jni::{
    objects::{GlobalRef, JValue},
    JNIEnv,
};
use std::{
    io::Write,
    os::unix::net::UnixStream,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};

struct Capture<'a> {
    framework: &'a Framework,
    record: GlobalRef,
    policy: Option<GlobalRef>,
}
impl Drop for Capture<'_> {
    fn drop(&mut self) {
        if let Ok(mut env) = self.framework.vm.attach_current_thread() {
            let _ = env.exception_clear();
            let _ = env.call_method(self.record.as_obj(), "stop", "()V", &[]);
            let _ = env.call_method(self.record.as_obj(), "release", "()V", &[]);
            if let Some(policy) = &self.policy {
                let _ = env.call_static_method(
                    "android/media/AudioManager",
                    "unregisterAudioPolicyAsyncStatic",
                    "(Landroid/media/audiopolicy/AudioPolicy;)V",
                    &[JValue::Object(policy.as_obj())],
                );
            }
        }
    }
}

fn capture<'a>(env: &mut JNIEnv, framework: &'a Framework, duplicate: bool) -> Result<Capture<'a>> {
    env.with_local_frame(48, |env| -> Result<_> {
        let builder = env.new_object("android/media/AudioFormat$Builder", "()V", &[])?;
        let format_sig = "(I)Landroid/media/AudioFormat$Builder;";
        env.call_method(&builder, "setEncoding", format_sig, &[JValue::Int(2)])?;
        env.call_method(&builder, "setSampleRate", format_sig, &[JValue::Int(48000)])?;
        env.call_method(&builder, "setChannelMask", format_sig, &[JValue::Int(12)])?;
        let format = env.call_method(builder, "build", "()Landroid/media/AudioFormat;", &[])?.l()?;
        let (record, policy) = if duplicate {
            let rule = env.new_object("android/media/audiopolicy/AudioMixingRule$Builder", "()V", &[])?;
            for usage in [0, 1, 14] {
                let attributes = env.new_object("android/media/AudioAttributes$Builder", "()V", &[])?;
                env.call_method(&attributes, "setUsage", "(I)Landroid/media/AudioAttributes$Builder;", &[JValue::Int(usage)])?;
                let attributes = env.call_method(attributes, "build", "()Landroid/media/AudioAttributes;", &[])?.l()?;
                env.call_method(&rule, "addRule", "(Landroid/media/AudioAttributes;I)Landroid/media/audiopolicy/AudioMixingRule$Builder;", &[JValue::Object(&attributes),JValue::Int(1)])?;
            }
            let rule = env.call_method(rule, "build", "()Landroid/media/audiopolicy/AudioMixingRule;", &[])?.l()?;
            let mix = env.new_object("android/media/audiopolicy/AudioMix$Builder", "(Landroid/media/audiopolicy/AudioMixingRule;)V", &[JValue::Object(&rule)])?;
            env.call_method(&mix, "setFormat", "(Landroid/media/AudioFormat;)Landroid/media/audiopolicy/AudioMix$Builder;", &[JValue::Object(&format)])?;
            env.call_method(&mix, "setRouteFlags", "(I)Landroid/media/audiopolicy/AudioMix$Builder;", &[JValue::Int(3)])?;
            let mix = env.call_method(mix, "build", "()Landroid/media/audiopolicy/AudioMix;", &[])?.l()?;
            let policy = env.new_object("android/media/audiopolicy/AudioPolicy$Builder", "(Landroid/content/Context;)V", &[JValue::Object(framework.context())])?;
            env.call_method(&policy, "addMix", "(Landroid/media/audiopolicy/AudioMix;)Landroid/media/audiopolicy/AudioPolicy$Builder;", &[JValue::Object(&mix)])?;
            let policy = env.call_method(policy, "build", "()Landroid/media/audiopolicy/AudioPolicy;", &[])?.l()?;
            let code = env.call_static_method("android/media/AudioManager", "registerAudioPolicyStatic", "(Landroid/media/audiopolicy/AudioPolicy;)I", &[JValue::Object(&policy)])?.i()?;
            if code != 0 { bail!("Android denied playback duplication (audio policy status {code})"); }
            let record = env.call_method(&policy, "createAudioRecordSink", "(Landroid/media/audiopolicy/AudioMix;)Landroid/media/AudioRecord;", &[JValue::Object(&mix)])?.l()?;
            (record,Some(env.new_global_ref(policy)?))
        } else {
            let record = env.new_object("android/media/AudioRecord$Builder", "()V", &[])?;
            env.call_method(&record, "setContext", "(Landroid/content/Context;)Landroid/media/AudioRecord$Builder;", &[JValue::Object(framework.context())])?;
            env.call_method(&record, "setAudioSource", "(I)Landroid/media/AudioRecord$Builder;", &[JValue::Int(8)])?;
            env.call_method(&record, "setAudioFormat", "(Landroid/media/AudioFormat;)Landroid/media/AudioRecord$Builder;", &[JValue::Object(&format)])?;
            let minimum = env.call_static_method("android/media/AudioRecord", "getMinBufferSize", "(III)I",
                &[JValue::Int(48000), JValue::Int(12), JValue::Int(2)])?.i()?;
            if !(4..=1024*1024).contains(&minimum) { bail!("Invalid Android capture buffer size: {minimum}"); }
            let capacity = minimum.max(3840);
            env.call_method(&record, "setBufferSizeInBytes", "(I)Landroid/media/AudioRecord$Builder;", &[JValue::Int(capacity)])?;
            (env.call_method(record, "build", "()Landroid/media/AudioRecord;", &[])?.l()?,None)
        };
        if record.is_null() { bail!("No playback capture record"); }
        let result = Capture {framework,record:env.new_global_ref(record)?,policy};
        env.call_method(result.record.as_obj(), "startRecording", "()V", &[])?;
        let state=env.call_method(result.record.as_obj(),"getRecordingState","()I",&[])?.i()?;
        if state!=3 {
            bail!("Android did not start playback capture: recording state {state}");
        }
        Ok(result)
    })
}

pub(crate) fn run(
    framework: Framework,
    mut stream: UnixStream,
    stop: &AtomicBool,
    mode: u8,
) -> Result<()> {
    let mut env = framework.vm.attach_current_thread()?;
    let capture = capture(&mut env, &framework, mode == 2)?;
    for value in [MAGIC, 48000, 2] {
        stream.write_all(&value.to_be_bytes())?;
    }
    // 120 stereo frames = 2.5 ms. Drain each available block immediately.
    let mut pcm = [0u8; 480];
    // SAFETY: pcm remains allocated and is never resized while the Java buffer exists.
    // Reads complete synchronously before the same native bytes are sent.
    let buffer = unsafe { env.new_direct_byte_buffer(pcm.as_mut_ptr(), pcm.len())? };
    println!("Audio: native PCM 48000 Hz stereo | mode {mode}");
    println!(
        "Audio recording state: {}",
        env.call_method(capture.record.as_obj(), "getRecordingState", "()I", &[])?
            .i()?
    );
    while !stop.load(Ordering::Acquire) {
        let length = env
            .call_method(
                capture.record.as_obj(),
                "read",
                "(Ljava/nio/ByteBuffer;II)I",
                &[
                    JValue::Object(&buffer),
                    JValue::Int(pcm.len() as i32),
                    JValue::Int(1),
                ],
            )?
            .i()?;
        if length < 0 {
            bail!("Playback capture read failed: {length}");
        }
        if length == 0 {
            thread::sleep(Duration::from_millis(2));
            continue;
        }
        let length = length as usize;
        if length > pcm.len() || !length.is_multiple_of(4) {
            bail!("Invalid PCM capture size");
        }
        stream.write_all(&(length as u32).to_be_bytes())?;
        stream.write_all(&pcm[..length])?;
    }
    Ok(())
}

#[cfg(feature = "qa")]
pub(crate) fn validation_tone(env: &mut JNIEnv) -> Result<()> {
    let mut pcm = Vec::with_capacity(48000 * 4 * 3);
    for i in 0..48000 * 3 {
        let sample = ((i as f64 * 440. * std::f64::consts::TAU / 48000.).sin() * 200.) as i16;
        for _ in 0..2 {
            pcm.extend_from_slice(&sample.to_le_bytes());
        }
    }
    let bytes = env.byte_array_from_slice(&pcm)?;
    let track = env.new_object(
        "android/media/AudioTrack",
        "(IIIIII)V",
        &[
            JValue::Int(3),
            JValue::Int(48000),
            JValue::Int(12),
            JValue::Int(2),
            JValue::Int(pcm.len() as i32),
            JValue::Int(0),
        ],
    )?;
    env.call_method(
        &track,
        "write",
        "([BII)I",
        &[
            JValue::Object(&bytes),
            JValue::Int(0),
            JValue::Int(pcm.len() as i32),
        ],
    )?;
    env.call_method(&track, "play", "()V", &[])?;
    thread::sleep(Duration::from_secs(3));
    env.call_method(&track, "stop", "()V", &[])?;
    env.call_method(track, "release", "()V", &[])?;
    Ok(())
}
