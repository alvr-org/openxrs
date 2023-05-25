use std::{ptr, sync::Arc};

use crate::*;

#[derive(Clone)]
pub struct FoveationProfileFB {
    inner: Arc<FoveationProfileFBInner>,
}

pub struct FoveationLevelProfile {
    pub level: FoveationLevelFB,
    pub vertical_offset: f32,
    pub dynamic: FoveationDynamicFB,
}

impl FoveationProfileFB {
    /// Take ownership of an existing foveation profile handle
    ///
    /// # Safety
    ///
    /// `handle` must be a valid foveation profile handle created with a [Session] associated with `instance`.
    #[inline]
    pub unsafe fn from_raw(instance: Instance, handle: sys::FoveationProfileFB) -> Self {
        Self {
            inner: Arc::new(FoveationProfileFBInner { instance, handle }),
        }
    }

    #[inline]
    pub fn as_raw(&self) -> sys::FoveationProfileFB {
        self.inner.handle
    }
}

impl<G> Session<G> {
    pub fn create_foveation_profile(
        &self,
        level_profile: Option<FoveationLevelProfile>,
    ) -> Result<FoveationProfileFB> {
        let fp = self
            .instance()
            .exts()
            .fb_foveation
            .as_ref()
            .ok_or(sys::Result::ERROR_EXTENSION_NOT_PRESENT)?;

        let mut level_profile = level_profile.map(|lp| sys::FoveationLevelProfileCreateInfoFB {
            ty: sys::FoveationLevelProfileCreateInfoFB::TYPE,
            next: std::ptr::null_mut(),
            vertical_offset: lp.vertical_offset,
            level: lp.level,
            dynamic: lp.dynamic,
        });
        let next = if let Some(level_profile) = level_profile.as_mut() {
            level_profile as *mut _ as *mut _
        } else {
            std::ptr::null_mut()
        };

        let mut create_info = sys::FoveationProfileCreateInfoFB {
            ty: sys::FoveationProfileCreateInfoFB::TYPE,
            next,
        };
        let mut profile = sys::FoveationProfileFB::NULL;
        let res =
            unsafe { (fp.create_foveation_profile)(self.as_raw(), &mut create_info, &mut profile) };
        cvt(res)?;

        Ok(unsafe { FoveationProfileFB::from_raw(self.instance().clone(), profile) })
    }
}

impl<G: Graphics> Session<G> {
    pub fn create_swapchain_with_foveation(
        &self,
        info: &SwapchainCreateInfo<G>,
        flags: SwapchainCreateFoveationFlagsFB,
    ) -> Result<Swapchain<G>> {
        let foveation_info = sys::SwapchainCreateInfoFoveationFB {
            ty: sys::SwapchainCreateInfoFoveationFB::TYPE,
            next: ptr::null_mut(),
            flags,
        };

        let mut out = sys::Swapchain::NULL;
        let info = sys::SwapchainCreateInfo {
            ty: sys::SwapchainCreateInfo::TYPE,
            next: &foveation_info as *const _ as _,
            create_flags: info.create_flags,
            usage_flags: info.usage_flags,
            format: G::lower_format(info.format),
            sample_count: info.sample_count,
            width: info.width,
            height: info.height,
            face_count: info.face_count,
            array_size: info.array_size,
            mip_count: info.mip_count,
        };
        unsafe {
            cvt((self.instance().fp().create_swapchain)(
                self.as_raw(),
                &info,
                &mut out,
            ))?;
            Ok(Swapchain::from_raw(self.clone(), out))
        }
    }
}

impl<G: Graphics> Swapchain<G> {
    pub fn update_foveation(&self, profile: &FoveationProfileFB) -> Result<()> {
        let fp = self
            .instance()
            .exts()
            .fb_swapchain_update_state
            .as_ref()
            .ok_or(sys::Result::ERROR_EXTENSION_NOT_PRESENT)?;

        let info = sys::SwapchainStateFoveationFB {
            ty: sys::SwapchainStateFoveationFB::TYPE,
            next: ptr::null_mut(),
            flags: SwapchainStateFoveationFlagsFB::EMPTY,
            profile: profile.as_raw(),
        };

        unsafe { cvt((fp.update_swapchain)(self.as_raw(), &info as *const _ as _))? };

        Ok(())
    }
}

struct FoveationProfileFBInner {
    instance: Instance,
    handle: sys::FoveationProfileFB,
}

impl Drop for FoveationProfileFBInner {
    fn drop(&mut self) {
        if let Some(fp) = self.instance.exts().fb_foveation.as_ref() {
            unsafe { (fp.destroy_foveation_profile)(self.handle) };
        }
    }
}
