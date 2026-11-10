//! The orb on Metal: metal.msl (the port of metal.hlsl) compiled at run time, drawn as one
//! full-window triangle into a CAMetalLayer, or into an offscreen texture for a snapshot.

use std::ptr::NonNull;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_core_foundation::CGSize;
use objc2_core_graphics::{kCGColorSpaceSRGB, CGColorSpace};
use objc2_foundation::NSString;
use objc2_metal::{
    MTLClearColor, MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue,
    MTLCreateSystemDefaultDevice, MTLDevice, MTLLibrary, MTLLoadAction, MTLOrigin, MTLPixelFormat,
    MTLPrimitiveType, MTLRegion, MTLRenderCommandEncoder, MTLRenderPassDescriptor,
    MTLRenderPipelineDescriptor, MTLRenderPipelineState, MTLSize, MTLStorageMode, MTLStoreAction,
    MTLTexture, MTLTextureDescriptor, MTLTextureUsage,
};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};

const SHADER: &str = include_str!("metal.msl");

/// The orb window and the drop inside it (logical px), as on Windows.
pub const WIN: f32 = 68.0;
pub const ORB: f32 = 48.0;
/// The orb renders at 2x its pixels and Core Animation filters it down.
pub const SS: f32 = 2.0;

/// The shader's uniforms, in metal.msl's order.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Params {
    pub orb_origin: [f32; 2],
    pub orb_size: f32,
    pub time: f32,
    pub angle: f32,
    pub distortion: f32,
    pub opacity: f32,
    pub brightness: f32,
    pub softness: f32,
    pub repetition: f32,
    pub shift_red: f32,
    pub shift_blue: f32,
    pub contour: f32,
    pub tint_a: f32,
    pub ring_start: f32,
    pub squash: f32,
    pub ring_mode: f32,
    pub ring_progress: f32,
    pub ring_radius: f32,
    pub px1: f32,
    pub wander: [f32; 2],
    pub warp: f32,
    pub pad: f32,
    pub ring_ink: [f32; 4],
    pub ring_track: [f32; 4],
    pub ring_halo: [f32; 4],
}

/// The resting drop at `k` device pixels per logical pixel: the same material values as
/// orb.rs (time 1.7, distortion 0.1, softness 0.12, repetition 1.5, ...).
pub fn rest_params(k: f32, angle: f32, brightness: f32, opacity: f32, drop_scale: f32) -> Params {
    let size = (WIN * k).round();
    let drop = ORB * k * drop_scale;
    Params {
        orb_origin: [(size - drop) / 2.0, (size - drop) / 2.0],
        orb_size: drop,
        time: 1.7,
        angle,
        distortion: 0.1,
        opacity,
        brightness,
        softness: 0.12,
        repetition: 1.5,
        shift_red: 0.15,
        shift_blue: 0.15,
        contour: 0.4,
        tint_a: 0.4,
        ring_radius: 30.0 * k,
        px1: k,
        ..Default::default()
    }
}

pub struct Gpu {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    pso: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
}

impl Gpu {
    pub fn new() -> Result<Gpu, String> {
        let device = MTLCreateSystemDefaultDevice().ok_or("no Metal device")?;
        let lib = device
            .newLibraryWithSource_options_error(&NSString::from_str(SHADER), None)
            .map_err(|e| format!("metal.msl: {}", e.localizedDescription()))?;
        let vs = lib
            .newFunctionWithName(&NSString::from_str("vs_main"))
            .ok_or("vs_main")?;
        let fs = lib
            .newFunctionWithName(&NSString::from_str("ps_main"))
            .ok_or("ps_main")?;
        let desc = MTLRenderPipelineDescriptor::new();
        desc.setVertexFunction(Some(&vs));
        desc.setFragmentFunction(Some(&fs));
        unsafe {
            desc.colorAttachments()
                .objectAtIndexedSubscript(0)
                .setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        }
        let pso = device
            .newRenderPipelineStateWithDescriptor_error(&desc)
            .map_err(|e| format!("pipeline: {}", e.localizedDescription()))?;
        let queue = device.newCommandQueue().ok_or("no command queue")?;
        Ok(Gpu { device, queue, pso })
    }

    /// A layer for the orb window, `px` device pixels square.
    pub fn layer(&self, px: f32) -> Retained<CAMetalLayer> {
        let layer = CAMetalLayer::new();
        layer.setDevice(Some(&self.device));
        layer.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        layer.setFramebufferOnly(true);
        layer.setOpaque(false);
        layer.setDrawableSize(CGSize {
            width: px as f64,
            height: px as f64,
        });
        let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }));
        layer.setColorspace(space.as_deref());
        layer
    }

    fn encode(
        &self,
        target: &ProtocolObject<dyn MTLTexture>,
        p: &Params,
    ) -> Option<Retained<ProtocolObject<dyn MTLCommandBuffer>>> {
        let rpd = MTLRenderPassDescriptor::new();
        let ca = unsafe { rpd.colorAttachments().objectAtIndexedSubscript(0) };
        ca.setTexture(Some(target));
        ca.setLoadAction(MTLLoadAction::Clear);
        ca.setStoreAction(MTLStoreAction::Store);
        ca.setClearColor(MTLClearColor {
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            alpha: 0.0,
        });
        let cb = self.queue.commandBuffer()?;
        let enc = cb.renderCommandEncoderWithDescriptor(&rpd)?;
        enc.setRenderPipelineState(&self.pso);
        unsafe {
            enc.setFragmentBytes_length_atIndex(
                NonNull::from(p).cast(),
                std::mem::size_of::<Params>(),
                0,
            );
            enc.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 3);
        }
        enc.endEncoding();
        Some(cb)
    }

    /// Draws one frame into the layer and presents it.
    pub fn draw(&self, layer: &CAMetalLayer, p: &Params) {
        let Some(drawable) = layer.nextDrawable() else {
            return;
        };
        if let Some(cb) = self.encode(&drawable.texture(), p) {
            cb.presentDrawable(ProtocolObject::from_ref(&*drawable));
            cb.commit();
        }
    }

    /// One frame offscreen: premultiplied RGBA rows, `px` square.
    pub fn render(&self, px: usize, p: &Params) -> Option<Vec<u8>> {
        let td = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::BGRA8Unorm,
                px,
                px,
                false,
            )
        };
        td.setUsage(MTLTextureUsage::RenderTarget);
        td.setStorageMode(MTLStorageMode::Shared);
        let tex = self.device.newTextureWithDescriptor(&td)?;
        let cb = self.encode(&tex, p)?;
        cb.commit();
        cb.waitUntilCompleted();
        let mut out = vec![0u8; px * px * 4];
        unsafe {
            tex.getBytes_bytesPerRow_fromRegion_mipmapLevel(
                NonNull::new(out.as_mut_ptr()).unwrap().cast(),
                px * 4,
                MTLRegion {
                    origin: MTLOrigin { x: 0, y: 0, z: 0 },
                    size: MTLSize {
                        width: px,
                        height: px,
                        depth: 1,
                    },
                },
                0,
            );
        }
        for c in out.chunks_mut(4) {
            c.swap(0, 2);
        }
        Some(out)
    }
}

/// The resting orb at `scale`, without supersampling (as the window's pixels after filtering).
pub fn snapshot(angle: f32, brightness: f32, scale: f32) -> Option<(u32, u32, Vec<u8>)> {
    let gpu = Gpu::new().map_err(|e| eprintln!("{e}")).ok()?;
    let px = (WIN * scale).round() as usize;
    let p = rest_params(scale, angle, brightness, 1.0, 1.0);
    gpu.render(px, &p).map(|v| (px as u32, px as u32, v))
}
