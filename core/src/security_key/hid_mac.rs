//! Security keys on USB on the Mac: IOKit's HID manager, matching the FIDO
//! Alliance's usage page (0xF1D0), as browsers do. The app isn't sandboxed
//! (the CI checks), and a security key isn't a keyboard, so macOS asks
//! nothing. A key's input reports arrive on a callback on the run loop of
//! the thread asking it, which runs while that thread waits for one.

use std::collections::VecDeque;
use std::ffi::{c_char, c_void};
use std::sync::Mutex;
use std::time::Duration;

use super::ctap::Hid;
use crate::error::{Error, Result};

type CFTypeRef = *const c_void;
type CFIndex = isize;
type IOReturn = i32;
type ReportCallback = extern "C" fn(
    context: *mut c_void,
    result: IOReturn,
    sender: *mut c_void,
    kind: u32,
    id: u32,
    report: *mut u8,
    length: CFIndex,
);

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFAllocatorDefault: CFTypeRef;
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
    static kCFRunLoopDefaultMode: CFTypeRef;
    fn CFDictionaryCreateMutable(
        allocator: CFTypeRef,
        capacity: CFIndex,
        keys: *const c_void,
        values: *const c_void,
    ) -> CFTypeRef;
    fn CFDictionarySetValue(dictionary: CFTypeRef, key: CFTypeRef, value: CFTypeRef);
    fn CFNumberCreate(allocator: CFTypeRef, kind: CFIndex, value: *const c_void) -> CFTypeRef;
    fn CFStringCreateWithCString(
        allocator: CFTypeRef,
        text: *const c_char,
        encoding: u32,
    ) -> CFTypeRef;
    fn CFSetGetCount(set: CFTypeRef) -> CFIndex;
    fn CFSetGetValues(set: CFTypeRef, values: *mut CFTypeRef);
    fn CFRetain(object: CFTypeRef) -> CFTypeRef;
    fn CFRelease(object: CFTypeRef);
    fn CFRunLoopGetCurrent() -> CFTypeRef;
    fn CFRunLoopRunInMode(mode: CFTypeRef, seconds: f64, after_one: u8) -> i32;
}

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOHIDManagerCreate(allocator: CFTypeRef, options: u32) -> CFTypeRef;
    fn IOHIDManagerSetDeviceMatching(manager: CFTypeRef, matching: CFTypeRef);
    fn IOHIDManagerCopyDevices(manager: CFTypeRef) -> CFTypeRef;
    fn IOHIDDeviceOpen(device: CFTypeRef, options: u32) -> IOReturn;
    fn IOHIDDeviceClose(device: CFTypeRef, options: u32) -> IOReturn;
    fn IOHIDDeviceSetReport(
        device: CFTypeRef,
        kind: u32,
        id: CFIndex,
        report: *const u8,
        length: CFIndex,
    ) -> IOReturn;
    fn IOHIDDeviceRegisterInputReportCallback(
        device: CFTypeRef,
        report: *mut u8,
        length: CFIndex,
        callback: Option<ReportCallback>,
        context: *mut c_void,
    );
    fn IOHIDDeviceScheduleWithRunLoop(device: CFTypeRef, run_loop: CFTypeRef, mode: CFTypeRef);
    fn IOHIDDeviceUnscheduleFromRunLoop(device: CFTypeRef, run_loop: CFTypeRef, mode: CFTypeRef);
}

const UTF8: u32 = 0x0800_0100;
const SINT32: CFIndex = 3;
const OUTPUT_REPORT: u32 = 1;

/// A CF object the core made, released when dropped.
struct Owned(CFTypeRef);

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the core holds this reference and releases it once.
            unsafe { CFRelease(self.0) };
        }
    }
}

fn cf_string(text: &str) -> Owned {
    let text = std::ffi::CString::new(text).expect("no NUL");
    // SAFETY: a NUL-terminated string, valid for the call.
    Owned(unsafe { CFStringCreateWithCString(kCFAllocatorDefault, text.as_ptr(), UTF8) })
}

fn cf_number(n: i32) -> Owned {
    // SAFETY: a 32-bit integer, valid for the call.
    Owned(unsafe {
        CFNumberCreate(
            kCFAllocatorDefault,
            SINT32,
            &n as *const i32 as *const c_void,
        )
    })
}

/// A security key plugged in, not yet opened.
pub struct Found(Owned);

// SAFETY: an IOHIDDevice may be opened and used from any one thread at a time.
unsafe impl Send for Found {}

/// The security keys plugged in.
pub fn devices() -> Vec<Found> {
    // SAFETY: each CF call gets valid objects the core made, and releases
    // what it was given (Owned), except the set's members, retained first.
    unsafe {
        let manager = Owned(IOHIDManagerCreate(kCFAllocatorDefault, 0));
        if manager.0.is_null() {
            return Vec::new();
        }
        let matching = Owned(CFDictionaryCreateMutable(
            kCFAllocatorDefault,
            2,
            &kCFTypeDictionaryKeyCallBacks as *const c_void,
            &kCFTypeDictionaryValueCallBacks as *const c_void,
        ));
        let (page_key, page) = (cf_string("PrimaryUsagePage"), cf_number(0xf1d0));
        let (usage_key, usage) = (cf_string("PrimaryUsage"), cf_number(0x01));
        CFDictionarySetValue(matching.0, page_key.0, page.0);
        CFDictionarySetValue(matching.0, usage_key.0, usage.0);
        IOHIDManagerSetDeviceMatching(manager.0, matching.0);
        let set = Owned(IOHIDManagerCopyDevices(manager.0));
        if set.0.is_null() {
            return Vec::new();
        }
        let count = CFSetGetCount(set.0).max(0) as usize;
        let mut members: Vec<CFTypeRef> = vec![std::ptr::null(); count];
        CFSetGetValues(set.0, members.as_mut_ptr());
        members
            .into_iter()
            .filter(|d| !d.is_null())
            .map(|d| Found(Owned(CFRetain(d))))
            .collect()
    }
}

/// What a key sent, gathered by the callback.
struct Inbox {
    buffer: [u8; 64],
    reports: Mutex<VecDeque<[u8; 64]>>,
}

extern "C" fn report_arrived(
    context: *mut c_void,
    result: IOReturn,
    _sender: *mut c_void,
    _kind: u32,
    _id: u32,
    report: *mut u8,
    length: CFIndex,
) {
    if result != 0 || report.is_null() || length != 64 {
        return;
    }
    // SAFETY: the context is the Inbox `Device` keeps until it unregisters
    // this callback, and the report is `length` bytes IOKit owns for the call.
    let (inbox, report) = unsafe {
        (
            &*(context as *const Inbox),
            std::slice::from_raw_parts(report, 64),
        )
    };
    if let Ok(mut reports) = inbox.reports.lock() {
        reports.push_back(report.try_into().unwrap());
    }
}

/// An open key, scheduled on the run loop of the thread that opened it.
pub struct Device {
    device: Owned,
    run_loop: CFTypeRef,
    inbox: Box<Inbox>,
}

impl Device {
    pub fn open(found: Found) -> Result<Device> {
        let device = found.0;
        // SAFETY: a retained device; the callback's buffer and context live
        // in the boxed Inbox until Drop unregisters them.
        unsafe {
            if IOHIDDeviceOpen(device.0, 0) != 0 {
                return Err(Error::TwoStep(
                    "PassBundle couldn't reach the security key. Unplug it, plug it in again, and try again.",
                ));
            }
            let mut inbox = Box::new(Inbox {
                buffer: [0; 64],
                reports: Mutex::new(VecDeque::new()),
            });
            let context = &mut *inbox as *mut Inbox as *mut c_void;
            IOHIDDeviceRegisterInputReportCallback(
                device.0,
                inbox.buffer.as_mut_ptr(),
                64,
                Some(report_arrived),
                context,
            );
            let run_loop = CFRunLoopGetCurrent();
            IOHIDDeviceScheduleWithRunLoop(device.0, run_loop, kCFRunLoopDefaultMode);
            Ok(Device {
                device,
                run_loop,
                inbox,
            })
        }
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        // SAFETY: undoes `open` on the same thread, before the Inbox goes.
        unsafe {
            IOHIDDeviceRegisterInputReportCallback(
                self.device.0,
                self.inbox.buffer.as_mut_ptr(),
                64,
                None,
                std::ptr::null_mut(),
            );
            IOHIDDeviceUnscheduleFromRunLoop(self.device.0, self.run_loop, kCFRunLoopDefaultMode);
            IOHIDDeviceClose(self.device.0, 0);
        }
    }
}

fn unplugged() -> Error {
    Error::TwoStep("The security key was unplugged. Plug it in again.")
}

impl Hid for Device {
    fn write(&mut self, report: &[u8; 64]) -> Result<()> {
        // SAFETY: 64 bytes, valid for the call.
        let result =
            unsafe { IOHIDDeviceSetReport(self.device.0, OUTPUT_REPORT, 0, report.as_ptr(), 64) };
        if result == 0 {
            Ok(())
        } else {
            Err(unplugged())
        }
    }

    fn read(&mut self, timeout: Duration) -> Result<Option<[u8; 64]>> {
        let next = |inbox: &Inbox| inbox.reports.lock().ok().and_then(|mut r| r.pop_front());
        if let Some(report) = next(&self.inbox) {
            return Ok(Some(report));
        }
        // SAFETY: runs this thread's own run loop, where the device is scheduled.
        unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, timeout.as_secs_f64(), 1) };
        Ok(next(&self.inbox))
    }
}
