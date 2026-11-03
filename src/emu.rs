//! The emulator's gRPC API (ADR 0002): screen frames for the view, and its touches and keys.

use crate::{R, supervisor};
use std::sync::Arc;
use std::time::Duration;
use tonic::metadata::MetadataValue;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;

#[allow(clippy::all, clippy::pedantic)]
mod pb {
    tonic::include_proto!("android.emulation.control");
}

use pb::emulator_controller_client::EmulatorControllerClient;
use pb::image_format::ImgFormat;
use pb::keyboard_event::KeyEventType;
use pb::{ImageFormat, KeyboardEvent, Touch, TouchEvent};

/// Adds the emulator's token to every call (`-grpc-use-token`).
#[derive(Clone)]
struct Auth(MetadataValue<tonic::metadata::Ascii>);

impl tonic::service::Interceptor for Auth {
    fn call(&mut self, mut req: tonic::Request<()>) -> Result<tonic::Request<()>, tonic::Status> {
        req.metadata_mut().insert("authorization", self.0.clone());
        Ok(req)
    }
}

type Client = EmulatorControllerClient<InterceptedService<Channel, Auth>>;

/// A connection to one emulator; cheap to clone across threads.
#[derive(Clone)]
pub struct Emu {
    rt: Arc<tokio::runtime::Runtime>,
    client: Client,
}

/// One RGB888 frame.
pub struct Frame {
    pub w: usize,
    pub h: usize,
    pub rgb: Vec<u8>,
}

impl Emu {
    pub fn connect(avd: &str) -> R<Emu> {
        let (port, token) = supervisor::grpc_endpoint(avd)?;
        let rt = Arc::new(tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?);
        let header = format!("Bearer {token}").parse()?;
        let channel = rt.block_on(
            Channel::from_shared(format!("http://127.0.0.1:{port}"))?.connect_timeout(Duration::from_secs(5)).tcp_nodelay(true).connect(),
        )?;
        let client = EmulatorControllerClient::with_interceptor(channel, Auth(header)).max_decoding_message_size(64 * 1024 * 1024);
        Ok(Emu { rt, client })
    }

    /// Stream frames about `width` px wide to `on_frame` until the emulator goes or it returns false.
    /// Calls `done` with the reason when the stream ends.
    pub fn stream(&self, width: u32, mut on_frame: impl FnMut(Frame) -> bool + Send + 'static, done: impl FnOnce(String) + Send + 'static) {
        let mut client = self.client.clone();
        self.rt.spawn(async move {
            let fmt = ImageFormat { format: ImgFormat::Rgb888 as i32, width, ..Default::default() };
            let reason = match client.stream_screenshot(fmt).await {
                // The emulator's auth errors quote the token; keep it off the screen.
                Err(e) if e.code() == tonic::Code::Unauthenticated => "screen stream failed: the emulator rejected its gRPC token".into(),
                Err(e) => format!("screen stream failed: {}", e.message()),
                Ok(resp) => {
                    let mut s = resp.into_inner();
                    loop {
                        match s.message().await {
                            Ok(Some(img)) => {
                                let f = img.format.unwrap_or_default();
                                let (w, h) = (f.width as usize, f.height as usize);
                                if w * h * 3 == img.image.len() && !on_frame(Frame { w, h, rgb: img.image }) {
                                    break String::new();
                                }
                            }
                            Ok(None) => break "screen stream ended".into(),
                            Err(e) => break format!("screen stream ended: {}", e.message()),
                        }
                    }
                }
            };
            done(reason);
        });
    }

    fn touch(&self, x: f64, y: f64, pressure: i32) -> R<()> {
        let ev = TouchEvent { touches: vec![Touch { x: x as i32, y: y as i32, identifier: 0, pressure, ..Default::default() }], display: 0 };
        let mut c = self.client.clone();
        self.rt.block_on(async { tokio::time::timeout(Duration::from_secs(5), c.send_touch(ev)).await })??;
        Ok(())
    }

    pub fn tap(&self, x: f64, y: f64) -> R<()> {
        self.touch(x, y, 1)?;
        std::thread::sleep(Duration::from_millis(40));
        self.touch(x, y, 0)
    }

    /// Press at `from`, move to `to` over about `ms`, release.
    pub fn drag(&self, from: (f64, f64), to: (f64, f64), ms: u64) -> R<()> {
        let steps = (ms / 16).max(2);
        self.touch(from.0, from.1, 1)?;
        for i in 1..=steps {
            let k = i as f64 / steps as f64;
            self.touch(from.0 + (to.0 - from.0) * k, from.1 + (to.1 - from.1) * k, 1)?;
            std::thread::sleep(Duration::from_millis(16));
        }
        self.touch(to.0, to.1, 0)
    }

    fn send_key(&self, ev: KeyboardEvent) -> R<()> {
        let mut c = self.client.clone();
        self.rt.block_on(async { tokio::time::timeout(Duration::from_secs(5), c.send_key(ev)).await })??;
        Ok(())
    }

    /// A named key (W3C key values: "Enter", "Backspace", "Tab", "GoBack", "GoHome", "AppSwitch").
    pub fn key(&self, key: &str) -> R<()> {
        self.send_key(KeyboardEvent { event_type: KeyEventType::Keypress as i32, key: key.into(), ..Default::default() })
    }

    /// Text as typed on a keyboard, any script.
    pub fn text(&self, text: &str) -> R<()> {
        self.send_key(KeyboardEvent { event_type: KeyEventType::Keypress as i32, text: text.into(), ..Default::default() })
    }
}
