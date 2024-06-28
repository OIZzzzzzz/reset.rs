// SPDX-License-Identifier: GPL-2.0

//! Support for reset controller drivers.
//!
//! C header: [`include/linux/gpio/driver.h`](../../../../include/linux/reset-controller.h)

use crate::{
    bindings,
    device::{self,RawDevice},
    error::{code::*, Error, Result, from_result},
    pr_err,
    pr_warn,
    platform,
    types::{Opaque, ForeignOwnable},
};

use core::{ 
    cell::UnsafeCell, 
    ffi::c_void,
    marker::{PhantomData, PhantomPinned}, 
    pin::Pin,
};

use macros::vtable;

/// Wraps the kernel's `struct reset_controller_dev`.
///
/// # Invariants
///
/// The pointer is non-null and valid, and has a non-zero reference count..
#[repr(transparent)]
pub struct ResetDevice(pub(crate) Opaque<bindings::reset_controller_dev>);

impl ResetDevice {
    /// Creates a reference to a [`ResetDevice`] from a valid pointer.
    ///
    /// # Safety
    ///
    /// Callers must ensure that `ptr` is valid, non-null, and has a non-zero reference count for
    /// the entire duration when the returned reference exists.
    pub unsafe fn from_raw<'a>(ptr: *mut bindings::reset_controller_dev) -> &'a Self {
        // SAFETY: Guaranteed by the safety requirements of the function.
        unsafe { &*ptr.cast() }
    }

    /// Returns a raw pointer to the inner C struct.
    #[inline]
    pub fn as_ptr(&self) -> *mut bindings::reset_controller_dev {
        self.0.get()
    }
}

/// A registration of a reset controller.
pub struct ResetRegistration<T: ResetDriverOps> {
    rcdev: UnsafeCell<bindings::reset_controller_dev>,
    dev: Option<device::Device>,
    registered: bool,
    _p: PhantomData<T>,
    _pin: PhantomPinned,
}

impl <T: ResetDriverOps> Drop  for ResetRegistration<T> {
    fn drop(&mut self) {
        // Free data as well.
        // SAFETY: `data_pointer` was returned by `into_foreign` during registration.
        pr_err!("reset controller dropped.\n")
    }
}

impl<T: ResetDriverOps> ResetRegistration<T> {
    /// Creates a new [`ResetRegistration`] but does not register it yet.
    ///
    /// It is allowed to move.
    pub fn new() -> Self {
        Self {
            rcdev: UnsafeCell::new(bindings::reset_controller_dev::default()),
            dev: None,
            registered: false,
            _pin: PhantomPinned,
            _p: PhantomData,
        }
    }

    /// Registers a reset controller with the rest of the kernel.
    /// 
    /// use `devm_reset_controller_register` to register this device.
    pub fn register(
        self: Pin<&mut Self>,
        dev:  &mut platform::Device,
        nr_resets: u32,
        data: T::Data,
    ) -> Result {
        // SAFETY: We never move out of `this`.
        let this = unsafe { self.get_unchecked_mut() };
        if this.registered {
            pr_warn!("Reset controller is already registered\n");
            return Err(EINVAL);
        }
        
        let rcdev = this.rcdev.get_mut();

        rcdev.dev = dev.raw_device();
        rcdev.nr_resets = nr_resets;
        rcdev.of_node = unsafe {(*rcdev.dev).of_node};
        rcdev.ops = Adapter::<T>::build();

        let data_pointer = <T::Data as ForeignOwnable>::into_foreign(data) as *mut c_void;

        unsafe { bindings::dev_set_drvdata(rcdev.dev, data_pointer)};
        let ret: i32 = unsafe { bindings::devm_reset_controller_register(rcdev.dev, this.rcdev.get()) };
        if ret < 0 {
            // SAFETY: `data_pointer` was returned by `into_foreign` above.
            unsafe { T::Data::from_foreign(data_pointer) };
            return Err(Error::from_errno(ret));
        }
        
        this.dev = Some(device::Device::from_dev(dev));
        this.registered = true;
        Ok(())
    }
}

// SAFETY: `Registration` doesn't offer any methods or access to fields when shared between threads
// or CPUs, so it is safe to share it.
unsafe impl<T: ResetDriverOps> Sync for ResetRegistration<T> {}

// SAFETY: Registration with and unregistration from the gpio subsystem can happen from any thread.
// Additionally, `T::Data` (which is dropped during unregistration) is `Send`, so it is ok to move
// `Registration` to different threads.
#[allow(clippy::non_send_fields_in_send_ty)]
unsafe impl<T: ResetDriverOps> Send for ResetRegistration<T> {}

/// Registers a gpio chip with the rest of the kernel.
///
/// It automatically defines the required lock classes.
#[macro_export]
macro_rules! reset_controller_register {
    ($reg:expr, $dev:expr, $nr_resets:expr, $data:expr $(,)?) => {{
        $crate::reset::ResetRegistration::register(
            $reg,
            $dev,
            $nr_resets,
            $data,
        )
    }};
}

/// Reset controller's operations
#[vtable]
pub trait ResetDriverOps {
    /// User data that will be accessible to all operations
    type Data: ForeignOwnable + Send + Sync ;

    /// for self-deasserting resets, does all necessary things to reset the device
    fn reset(_data:<Self::Data as ForeignOwnable>::Borrowed<'_> , _id: u64) -> Result<i32> {
        Err(ENOTSUPP)
    }

    /// manually assert the reset line, if supported
    fn assert(_data: <Self::Data as ForeignOwnable>::Borrowed<'_>, _id: u64) -> Result<i32> {
        Err(ENOTSUPP)
    }

    /// manually deassert the reset line, if supported
    fn deassert(_data: <Self::Data as ForeignOwnable>::Borrowed<'_>, _id: u64) -> Result<i32> {
        Err(ENOTSUPP)
    }

    /// return the status of the reset line, if supported
    fn status(_data: <Self::Data as ForeignOwnable>::Borrowed<'_>, _id: u64) -> Result<i32> {
        Err(ENOTSUPP)
    }
}

pub(crate) struct Adapter<T:ResetDriverOps>(PhantomData<T>);

impl<T: ResetDriverOps> Adapter<T> {
    /// Returns Static Reference to the C ops struct.
    fn build() -> &'static bindings::reset_control_ops {
        &Self::VTABLE
    }

    /// Reset Control Operations Vtable
    const VTABLE: bindings::reset_control_ops = bindings::reset_control_ops {
        reset: if T::HAS_RESET {
            Some(Adapter::<T>::reset_callback)
        } else {
            None
        },
        assert: if T::HAS_ASSERT {
            Some(Adapter::<T>::assert_callback)
        } else {
            None
        },
        deassert: if T::HAS_DEASSERT {
            Some(Adapter::<T>::deassert_callback)
        } else {
            None
        },
        status: if T::HAS_STATUS {
            Some(Adapter::<T>::status_callback)
        } else {
            None
        },
    };

    unsafe extern "C" fn reset_callback(
        rcdev: *mut bindings::reset_controller_dev,
        id: core::ffi::c_ulong,
    ) -> core::ffi::c_int {
        from_result(||{
            let data_pointer = unsafe { bindings::dev_get_drvdata((*rcdev).dev) };
            let data = unsafe { T::Data::borrow(data_pointer) };
            let v = T::reset(data, id)?;
            Ok(v as _)
        })
    }

    unsafe extern "C" fn assert_callback(
        rcdev: *mut bindings::reset_controller_dev,
        id: core::ffi::c_ulong,
    ) -> core::ffi::c_int {
        from_result(||{
            let data_pointer = unsafe { bindings::dev_get_drvdata((*rcdev).dev) };
            let data = unsafe { T::Data::borrow(data_pointer) };
            let v = T::assert(data, id)?;
            Ok(v as _)
        })
    }

    unsafe extern "C" fn deassert_callback(
        rcdev: *mut bindings::reset_controller_dev,
        id: core::ffi::c_ulong,
    ) -> core::ffi::c_int {
        from_result(||{
            let data_pointer = unsafe { bindings::dev_get_drvdata((*rcdev).dev) };
            let data = unsafe { T::Data::borrow(data_pointer) };
            let v = T::deassert(data, id)?;
            Ok(v as _)
        })
    }

    unsafe extern "C" fn status_callback(
        rcdev: *mut bindings::reset_controller_dev,
        id: core::ffi::c_ulong,
    ) -> core::ffi::c_int {
        from_result(||{
            let data_pointer = unsafe { bindings::dev_get_drvdata((*rcdev).dev) };
            let data = unsafe { T::Data::borrow(data_pointer) };
            let v = T::status(data, id)?;
            Ok(v as _)
        })
    }
}
