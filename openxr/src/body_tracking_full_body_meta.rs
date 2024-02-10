use std::{ptr, sync::Arc};

use sys::{BodyJointFullBodyLocationMETA, BodyJointSetFullBodyMETA};

use crate::*;

pub const BODY_JOINT_COUNT_META: usize = 70;
pub const BODY_JOINT_FULL_BODY_COUNT_META: usize = 84;

pub struct BodyTrackerFullBodyMETA {
    pub(crate) session: Arc<session::SessionInner>,
    handle: sys::BodyTrackerFullBodyMETA,
}

impl BodyTrackerFullBodyMETA {
    #[inline]
    pub fn as_raw(&self) -> sys::BodyTrackerFullBodyMETA {
        self.handle
    }

    /// Take ownership of an existing body tracker
    ///
    /// # Safety
    ///
    /// `handle` must be a valid body tracker handle associated with `session`.
    #[inline]
    pub unsafe fn from_raw<G>(session: &Session<G>, handle: sys::BodyTrackerFullBodyMETA) -> Self {
        Self {
            handle,
            session: session.inner.clone(),
        }
    }

    #[inline]
    pub(crate) fn fp(&self) -> &raw::BodyTrackingFullBodyMETA {
        self.session
            .instance
            .exts()
            .meta_body_tracking_full_body
            .as_ref()
            .expect("Somehow created BodyTrackingFullBodyMETA without XR_META_body_tracking_full_body being enabled")
    }
}

impl<G> Session<G> {
    pub fn create_body_tracker_full_body_meta(&self, full_body: bool) -> Result<BodyTrackerFullBodyMETA> {
        let fp = self
            .inner
            .instance
            .exts()
            .meta_body_tracking_full_body
            .as_ref()
            .ok_or(sys::Result::ERROR_EXTENSION_NOT_PRESENT)?;

        let mut out = sys::BodyTrackerFullBodyMETA::NULL;
        let info = sys::BodyTrackerFullBodyCreateInfoMETA {
            ty: sys::BodyTrackerFullBodyCreateInfoMETA::TYPE,
            next: ptr::null(),
            body_joint_set: if full_body { BodyJointSetFullBodyMETA::FULL_BODY } else { BodyJointSetFullBodyMETA::DEFAULT },
        };
        let handle = unsafe {
            cvt((fp.create_body_tracker)(self.as_raw(), &info, &mut out))?;
            out
        };
        Ok(BodyTrackerFullBodyMETA {
            session: self.inner.clone(),
            handle,
        })
    }
}

impl Drop for BodyTrackerFullBodyMETA {
    fn drop(&mut self) {
        unsafe {
            (self.fp().destroy_body_tracker)(self.handle);
        }
    }
}

/// An array of `BodyJointFullBodyLocationMETA`s, one for each `FullBodyJointMETA`.
///
/// `FullBodyJointMETA`s can be used directly as an index for convenience.
pub type BodyJointFullBodyMETALocations = [BodyJointFullBodyLocationMETA; BODY_JOINT_FULL_BODY_COUNT_META];