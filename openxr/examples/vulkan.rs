//! Illustrates rendering using Vulkan with multiview. Supports any Vulkan 1.1 capable environment.
//!
//! Renders a smooth gradient across the entire view, with different colors per eye.
//!
//! This example uses minimal abstraction for clarity. Real-world code should encapsulate and
//! largely decouple its Vulkan and OpenXR components and handle errors gracefully.

use std::{
    io::Cursor,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use wgpu_hal as hal;

use ash::{
    util::read_spv,
    vk::{self, Handle},
};
use openxr as xr;

#[allow(clippy::field_reassign_with_default)] // False positive, might be fixed 1.51
#[cfg_attr(target_os = "android", ndk_glue::main)]
pub fn main() {
    // Handle interrupts gracefully
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::Relaxed);
    })
    .expect("setting Ctrl-C handler");

    #[cfg(feature = "static")]
    let entry = xr::Entry::linked();
    #[cfg(not(feature = "static"))]
    let entry = xr::Entry::load()
        .expect("couldn't find the OpenXR loader; try enabling the \"static\" feature");

    #[cfg(target_os = "android")]
    entry.initialize_android_loader().unwrap();

    // OpenXR will fail to initialize if we ask for an extension that OpenXR can't provide! So we
    // need to check all our extensions before initializing OpenXR with them. Note that even if the
    // extension is present, it's still possible you may not be able to use it. For example: the
    // hand tracking extension may be present, but the hand sensor might not be plugged in or turned
    // on. There are often additional checks that should be made before using certain features!
    let available_extensions = entry.enumerate_extensions().unwrap();

    // If a required extension isn't present, you want to ditch out here! It's possible something
    // like your rendering API might not be provided by the active runtime. APIs like OpenGL don't
    // have universal support.
    assert!(available_extensions.khr_vulkan_enable2);

    // Initialize OpenXR with the extensions we've found!
    let mut enabled_extensions = xr::ExtensionSet::default();
    enabled_extensions.khr_vulkan_enable2 = true;
    #[cfg(target_os = "android")]
    {
        enabled_extensions.khr_android_create_instance = true;
    }
    let xr_instance = entry
        .create_instance(
            &xr::ApplicationInfo {
                application_name: "openxrs example",
                application_version: 0,
                engine_name: "openxrs example",
                engine_version: 0,
            },
            &enabled_extensions,
            &[],
        )
        .unwrap();
    let instance_props = xr_instance.properties().unwrap();
    println!(
        "loaded OpenXR runtime: {} {}",
        instance_props.runtime_name, instance_props.runtime_version
    );

    // Request a form factor from the device (HMD, Handheld, etc.)
    let system = xr_instance
        .system(xr::FormFactor::HEAD_MOUNTED_DISPLAY)
        .unwrap();

    // Check what blend mode is valid for this device (opaque vs transparent displays). We'll just
    // take the first one available!
    let environment_blend_mode = xr_instance
        .enumerate_environment_blend_modes(system, VIEW_TYPE)
        .unwrap()[0];

    // OpenXR wants to ensure apps are using the correct graphics card and Vulkan features and
    // extensions, so the instance and device MUST be set up before Instance::create_session.

    let vk_target_version = vk::make_api_version(0, 1, 1, 0); // Vulkan 1.1 guarantees multiview support
    let vk_target_version_xr = xr::Version::new(1, 1, 0);

    let reqs = xr_instance
        .graphics_requirements::<xr::Vulkan>(system)
        .unwrap();

    if vk_target_version_xr < reqs.min_api_version_supported
        || vk_target_version_xr.major() > reqs.max_api_version_supported.major()
    {
        panic!(
            "OpenXR runtime requires Vulkan version > {}, < {}.0.0",
            reqs.min_api_version_supported,
            reqs.max_api_version_supported.major() + 1
        );
    }

    unsafe {
        let vk_entry = ash::Entry::load().unwrap();

        let vk_app_info = vk::ApplicationInfo::builder()
            .application_version(0)
            .engine_version(0)
            .api_version(vk_target_version);

        let mut flags = hal::InstanceFlags::empty();
        if cfg!(debug_assertions) {
            flags |= hal::InstanceFlags::VALIDATION;
            flags |= hal::InstanceFlags::DEBUG;
        }

        let vk_instance = {
            let extensions = <hal::api::Vulkan as hal::Api>::Instance::required_extensions(
                std::mem::transmute(vk_entry.static_fn().get_instance_proc_addr),
                flags,
            )
            .unwrap();
            let mut extensions_ptrs = extensions.iter().map(|x| x.as_ptr()).collect::<Vec<_>>();
            let layers = <hal::api::Vulkan as hal::Api>::Instance::required_layers(
                std::mem::transmute(vk_entry.static_fn().get_instance_proc_addr),
                flags,
            )
            .unwrap();
            let mut layers_ptrs = layers.iter().map(|x| x.as_ptr()).collect::<Vec<_>>();

            let vk_instance = xr_instance
                .create_vulkan_instance(
                    system,
                    std::mem::transmute(vk_entry.static_fn().get_instance_proc_addr),
                    &vk::InstanceCreateInfo::builder()
                        .application_info(&vk_app_info)
                        .enabled_extension_names(&extensions_ptrs)
                        .enabled_layer_names(&layers_ptrs) as *const _
                        as *const _,
                )
                .expect("XR error creating Vulkan instance")
                .map_err(vk::Result::from_raw)
                .expect("Vulkan error creating Vulkan instance");
            ash::Instance::load(
                vk_entry.static_fn(),
                vk::Instance::from_raw(vk_instance as _),
            )
        };
        let hal_instance = <hal::api::Vulkan as hal::Api>::Instance::from_raw(
            std::mem::transmute(vk_entry.static_fn().get_instance_proc_addr),
            vk_instance.handle().as_raw(),
            vk_target_version,
            flags,
            Some(Box::new(xr_instance.clone())),
        )
        .unwrap();

        let vk_physical_device = vk::PhysicalDevice::from_raw(
            xr_instance
                .vulkan_graphics_device(system, vk_instance.handle().as_raw() as _)
                .unwrap() as _,
        );
        let hal_exposed_adapter = hal_instance
            .expose_adapter(vk_physical_device.as_raw())
            .unwrap();

        let vk_device_properties = vk_instance.get_physical_device_properties(vk_physical_device);
        if vk_device_properties.api_version < vk_target_version {
            vk_instance.destroy_instance(None);
            panic!("Vulkan phyiscal device doesn't support version 1.1");
        }

        let queue_family_index = vk_instance
            .get_physical_device_queue_family_properties(vk_physical_device)
            .into_iter()
            .enumerate()
            .find_map(|(queue_family_index, info)| {
                if info.queue_flags.contains(vk::QueueFlags::GRAPHICS) {
                    Some(queue_family_index as u32)
                } else {
                    None
                }
            })
            .expect("Vulkan device has no graphics queue");

        let queue_index = 0;

        let features = wgpu::Features::SPIRV_SHADER_PASSTHROUGH;
        let limits = wgpu::Limits::default();

        let vk_device = {
            let (extensions, mut physical_features, _) = hal_exposed_adapter
                .adapter
                .required_device_capabilities(features, &limits);
            let mut extensions_ptrs = extensions.iter().map(|x| x.as_ptr()).collect::<Vec<_>>();

            let mut info = vk::DeviceCreateInfo::builder()
                .queue_create_infos(&[vk::DeviceQueueCreateInfo::builder()
                    .queue_family_index(queue_family_index)
                    .queue_priorities(&[1.0])
                    .build()])
                .enabled_extension_names(&extensions_ptrs)
                .build();
            physical_features.add_to_device_create_info(&mut info as *mut _ as _);

            let vk_device = xr_instance
                .create_vulkan_device(
                    system,
                    std::mem::transmute(vk_entry.static_fn().get_instance_proc_addr),
                    vk_physical_device.as_raw() as _,
                    &info as *const _ as *const _,
                )
                .expect("XR error creating Vulkan device")
                .map_err(vk::Result::from_raw)
                .expect("Vulkan error creating Vulkan device");

            ash::Device::load(vk_instance.fp_v1_0(), vk::Device::from_raw(vk_device as _))
        };
        let hal_device = hal_exposed_adapter
            .adapter
            .device_from_raw(
                vk_device.handle().as_raw(),
                true,
                features,
                &limits,
                queue_family_index,
                queue_index,
            )
            .unwrap();

        let wgpu_instance = wgpu::Instance::from_hal::<hal::api::Vulkan>(hal_instance);
        let wgpu_adapter = wgpu_instance.create_adapter_from_hal(hal_exposed_adapter);
        let (wgpu_device, wgpu_queue) = wgpu_adapter
            .create_device_from_hal(
                hal_device,
                &wgpu::DeviceDescriptor {
                    label: None,
                    features,
                    limits,
                },
                None,
            )
            .unwrap();

        let vertex_shader =
            wgpu_device.create_shader_module_spirv(&wgpu::ShaderModuleDescriptorSpirV {
                label: None,
                source: read_spv(&mut Cursor::new(&include_bytes!("fullscreen.vert.spv")[..]))
                    .unwrap()
                    .into(),
            });
        let fragment_shader =
            wgpu_device.create_shader_module_spirv(&wgpu::ShaderModuleDescriptorSpirV {
                label: None,
                source: read_spv(&mut Cursor::new(
                    &include_bytes!("debug_pattern.frag.spv")[..],
                ))
                .unwrap()
                .into(),
            });

        let pipeline_layout = wgpu_device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });

        let render_pipeline = wgpu_device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vertex_shader,
                entry_point: "main",
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0x0,
                alpha_to_coverage_enabled: false,
            },
            fragment: Some(wgpu::FragmentState {
                module: &fragment_shader,
                entry_point: "main",
                targets: &[wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    blend: None,
                    write_mask: wgpu::ColorWrites::RED
                        | wgpu::ColorWrites::GREEN
                        | wgpu::ColorWrites::BLUE,
                }],
            }),
            multiview: None,
        });

        // A session represents this application's desire to display things! This is where we hook
        // up our graphics API. This does not start the session; for that, you'll need a call to
        // Session::begin, which we do in 'main_loop below.
        let (session, mut frame_wait, mut frame_stream) = xr_instance
            .create_session::<xr::Vulkan>(
                system,
                &xr::vulkan::SessionCreateInfo {
                    instance: vk_instance.handle().as_raw() as _,
                    physical_device: vk_physical_device.as_raw() as _,
                    device: vk_device.handle().as_raw() as _,
                    queue_family_index,
                    queue_index: 0,
                },
            )
            .unwrap();

        // Create an action set to encapsulate our actions
        let action_set = xr_instance
            .create_action_set("input", "input pose information", 0)
            .unwrap();

        let right_action = action_set
            .create_action::<xr::Posef>("right_hand", "Right Hand Controller", &[])
            .unwrap();
        let left_action = action_set
            .create_action::<xr::Posef>("left_hand", "Left Hand Controller", &[])
            .unwrap();

        // Bind our actions to input devices using the given profile
        // If you want to access inputs specific to a particular device you may specify a different
        // interaction profile
        xr_instance
            .suggest_interaction_profile_bindings(
                xr_instance
                    .string_to_path("/interaction_profiles/khr/simple_controller")
                    .unwrap(),
                &[
                    xr::Binding::new(
                        &right_action,
                        xr_instance
                            .string_to_path("/user/hand/right/input/grip/pose")
                            .unwrap(),
                    ),
                    xr::Binding::new(
                        &left_action,
                        xr_instance
                            .string_to_path("/user/hand/left/input/grip/pose")
                            .unwrap(),
                    ),
                ],
            )
            .unwrap();

        // Attach the action set to the session
        session.attach_action_sets(&[&action_set]).unwrap();

        // Create an action space for each device we want to locate
        let right_space = right_action
            .create_space(session.clone(), xr::Path::NULL, xr::Posef::IDENTITY)
            .unwrap();
        let left_space = left_action
            .create_space(session.clone(), xr::Path::NULL, xr::Posef::IDENTITY)
            .unwrap();

        // OpenXR uses a couple different types of reference frames for positioning content; we need
        // to choose one for displaying our content! STAGE would be relative to the center of your
        // guardian system's bounds, and LOCAL would be relative to your device's starting location.
        let stage = session
            .create_reference_space(xr::ReferenceSpaceType::STAGE, xr::Posef::IDENTITY)
            .unwrap();

        // Main loop
        let mut swapchain = None;
        let mut event_storage = xr::EventDataBuffer::new();
        let mut session_running = false;
        // Index of the current frame, wrapped by PIPELINE_DEPTH. Not to be confused with the
        // swapchain image index.
        'main_loop: loop {
            if !running.load(Ordering::Relaxed) {
                println!("requesting exit");
                // The OpenXR runtime may want to perform a smooth transition between scenes, so we
                // can't necessarily exit instantly. Instead, we must notify the runtime of our
                // intent and wait for it to tell us when we're actually done.
                match session.request_exit() {
                    Ok(()) => {}
                    Err(xr::sys::Result::ERROR_SESSION_NOT_RUNNING) => break,
                    Err(e) => panic!("{}", e),
                }
            }

            while let Some(event) = xr_instance.poll_event(&mut event_storage).unwrap() {
                use xr::Event::*;
                match event {
                    SessionStateChanged(e) => {
                        // Session state change is where we can begin and end sessions, as well as
                        // find quit messages!
                        println!("entered state {:?}", e.state());
                        match e.state() {
                            xr::SessionState::READY => {
                                session.begin(VIEW_TYPE).unwrap();
                                session_running = true;
                            }
                            xr::SessionState::STOPPING => {
                                session.end().unwrap();
                                session_running = false;
                            }
                            xr::SessionState::EXITING | xr::SessionState::LOSS_PENDING => {
                                break 'main_loop;
                            }
                            _ => {}
                        }
                    }
                    InstanceLossPending(_) => {
                        break 'main_loop;
                    }
                    EventsLost(e) => {
                        println!("lost {} events", e.lost_event_count());
                    }
                    _ => {}
                }
            }

            if !session_running {
                // Don't grind up the CPU
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }

            // Block until the previous frame is finished displaying, and is ready for another one.
            // Also returns a prediction of when the next frame will be displayed, for use with
            // predicting locations of controllers, viewpoints, etc.
            let xr_frame_state = frame_wait.wait().unwrap();
            // Must be called before any rendering is done!
            frame_stream.begin().unwrap();

            if !xr_frame_state.should_render {
                frame_stream
                    .end(
                        xr_frame_state.predicted_display_time,
                        environment_blend_mode,
                        &[],
                    )
                    .unwrap();
                continue;
            }

            let swapchain = swapchain.get_or_insert_with(|| {
                // Now we need to find all the viewpoints we need to take care of! This is a
                // property of the view configuration type; in this example we use PRIMARY_STEREO,
                // so we should have 2 viewpoints.
                //
                // Because we are using multiview in this example, we require that all view
                // dimensions are identical.
                let views = xr_instance
                    .enumerate_view_configuration_views(system, VIEW_TYPE)
                    .unwrap();
                assert_eq!(views.len(), VIEW_COUNT as usize);
                assert_eq!(views[0], views[1]);

                // Create a swapchain for the viewpoints! A swapchain is a set of texture buffers
                // used for displaying to screen, typically this is a backbuffer and a front buffer,
                // one for rendering data to, and one for displaying on-screen.
                let resolution = vk::Extent2D {
                    width: views[0].recommended_image_rect_width,
                    height: views[0].recommended_image_rect_height,
                };
                let handle = session
                    .create_swapchain(&xr::SwapchainCreateInfo {
                        create_flags: xr::SwapchainCreateFlags::EMPTY,
                        usage_flags: xr::SwapchainUsageFlags::COLOR_ATTACHMENT
                            | xr::SwapchainUsageFlags::SAMPLED,
                        format: COLOR_FORMAT.as_raw() as _,
                        // The Vulkan graphics pipeline we create is not set up for multisampling,
                        // so we hardcode this to 1. If we used a proper multisampling setup, we
                        // could set this to `views[0].recommended_swapchain_sample_count`.
                        sample_count: 1,
                        width: resolution.width,
                        height: resolution.height,
                        face_count: 1,
                        array_size: 1,
                        mip_count: 1,
                    })
                    .unwrap();
                let swapchain = Arc::new(Mutex::new(handle));

                let hal_texture_desc = hal::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: resolution.width,
                        height: resolution.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    usage: hal::TextureUses::COLOR_TARGET | hal::TextureUses::RESOURCE,
                    memory_flags: hal::MemoryFlags::empty(),
                };

                let texture_desc = wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: resolution.width,
                        height: resolution.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                };

                // We'll want to track our own information about the swapchain, so we can draw stuff
                // onto it! We'll also create a buffer for each generated texture here as well.
                let images = swapchain.lock().unwrap().enumerate_images().unwrap();
                Swapchain {
                    handle: swapchain.clone(),
                    resolution,
                    buffers: images
                        .into_iter()
                        .map(|color_image| {
                            let color_image = vk::Image::from_raw(color_image);

                            let hal_texture = unsafe {
                                <hal::api::Vulkan as hal::Api>::Device::texture_from_raw(
                                    color_image.as_raw(),
                                    &hal_texture_desc,
                                    Some(Box::new(swapchain.clone())),
                                )
                            };

                            let wgpu_texture = unsafe {
                                wgpu_device.create_texture_from_hal::<hal::api::Vulkan>(
                                    hal_texture,
                                    &texture_desc,
                                )
                            };

                            let color = wgpu_texture.create_view(&wgpu::TextureViewDescriptor {
                                label: None,
                                format: None,
                                dimension: None,
                                aspect: wgpu::TextureAspect::All,
                                base_mip_level: 0,
                                mip_level_count: None,
                                base_array_layer: 0,
                                array_layer_count: None,
                            });

                            Framebuffer { color }
                        })
                        .collect(),
                }
            });

            // We need to ask which swapchain image to use for rendering! Which one will we get?
            // Who knows! It's up to the runtime to decide.
            let image_index = swapchain.handle.lock().unwrap().acquire_image().unwrap();

            // Wait until the image is available to render to. The compositor could still be
            // reading from it.
            swapchain
                .handle
                .lock()
                .unwrap()
                .wait_image(xr::Duration::INFINITE)
                .unwrap();

            let mut command_encoder = wgpu_device.create_command_encoder(&Default::default());

            let mut render_pass = command_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[wgpu::RenderPassColorAttachment {
                    view: &swapchain.buffers[image_index as usize].color,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: true,
                    },
                }],
                depth_stencil_attachment: None,
            });

            render_pass.set_viewport(
                0_f32,
                0_f32,
                swapchain.resolution.width as _,
                swapchain.resolution.width as _,
                0_f32,
                1_f32,
            );
            render_pass.set_scissor_rect(
                0,
                0,
                swapchain.resolution.width,
                swapchain.resolution.width,
            );

            render_pass.set_pipeline(&render_pipeline);
            render_pass.draw(0..3, 0..1);
            drop(render_pass);

            let command_buffer = command_encoder.finish();

            session.sync_actions(&[(&action_set).into()]).unwrap();

            // Find where our controllers are located in the Stage space
            let right_location = right_space
                .locate(&stage, xr_frame_state.predicted_display_time)
                .unwrap();

            let left_location = left_space
                .locate(&stage, xr_frame_state.predicted_display_time)
                .unwrap();

            let mut printed = false;
            if left_action.is_active(&session, xr::Path::NULL).unwrap() {
                print!(
                    "Left Hand: ({:0<12},{:0<12},{:0<12}), ",
                    left_location.pose.position.x,
                    left_location.pose.position.y,
                    left_location.pose.position.z
                );
                printed = true;
            }

            if right_action.is_active(&session, xr::Path::NULL).unwrap() {
                print!(
                    "Right Hand: ({:0<12},{:0<12},{:0<12})",
                    right_location.pose.position.x,
                    right_location.pose.position.y,
                    right_location.pose.position.z
                );
                printed = true;
            }
            if printed {
                println!();
            }

            // Fetch the view transforms. To minimize latency, we intentionally do this *after*
            // recording commands to render the scene, i.e. at the last possible moment before
            // rendering begins in earnest on the GPU. Uniforms dependent on this data can be sent
            // to the GPU just-in-time by writing them to per-frame host-visible memory which the
            // GPU will only read once the command buffer is submitted.
            let (_, views) = session
                .locate_views(VIEW_TYPE, xr_frame_state.predicted_display_time, &stage)
                .unwrap();

            wgpu_queue.submit(Some(command_buffer));
            swapchain.handle.lock().unwrap().release_image().unwrap();

            // Tell OpenXR what to present for this frame
            let rect = xr::Rect2Di {
                offset: xr::Offset2Di { x: 0, y: 0 },
                extent: xr::Extent2Di {
                    width: swapchain.resolution.width as _,
                    height: swapchain.resolution.height as _,
                },
            };

            let swapchain = &swapchain.handle.lock().unwrap();

            frame_stream
                .end(
                    xr_frame_state.predicted_display_time,
                    environment_blend_mode,
                    &[
                        &xr::CompositionLayerProjection::new().space(&stage).views(&[
                            xr::CompositionLayerProjectionView::new()
                                .pose(views[0].pose)
                                .fov(views[0].fov)
                                .sub_image(
                                    xr::SwapchainSubImage::new()
                                        .swapchain(swapchain)
                                        .image_array_index(0)
                                        .image_rect(rect),
                                ),
                            xr::CompositionLayerProjectionView::new()
                                .pose(views[1].pose)
                                .fov(views[1].fov)
                                .sub_image(
                                    xr::SwapchainSubImage::new()
                                        .swapchain(swapchain)
                                        .image_array_index(0)
                                        .image_rect(rect),
                                ),
                        ]),
                    ],
                )
                .unwrap();
        }

        // OpenXR MUST be allowed to clean up before we destroy Vulkan resources it could touch, so
        // first we must drop all its handles.
        drop((
            session,
            frame_wait,
            frame_stream,
            stage,
            action_set,
            left_space,
            right_space,
            swapchain,
        ));
    }

    #[cfg(target_os = "android")]
    ndk_glue::native_activity().finish();

    // graphics objects are dropped here
}

pub const COLOR_FORMAT: vk::Format = vk::Format::R8G8B8A8_SRGB;
pub const VIEW_COUNT: u32 = 2;
const VIEW_TYPE: xr::ViewConfigurationType = xr::ViewConfigurationType::PRIMARY_STEREO;

struct Swapchain {
    handle: Arc<Mutex<xr::Swapchain<xr::Vulkan>>>,
    buffers: Vec<Framebuffer>,
    resolution: vk::Extent2D,
}

struct Framebuffer {
    color: wgpu::TextureView,
}
