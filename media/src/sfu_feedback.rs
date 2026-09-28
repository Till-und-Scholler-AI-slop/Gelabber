//! Surface PLI/FIR to the SFU while keeping the default loss/report interceptors.
//! rtc 0.20.5's default chain consumes inbound RTCP before TrackLocal::poll;
//! its rtcp_processing_webrtc2webrtc integration test uses an outer interceptor
//! for this same purpose. Only keyframe feedback needs application forwarding.

use std::collections::VecDeque;

use rtc::interceptor::{Interceptor, Packet, StreamInfo, TaggedPacket, interceptor};
use rtc::sansio;
use rtc::shared::error::Error;

#[derive(Interceptor)]
pub(super) struct KeyframeFeedback<P> {
    #[next]
    next: P,
    feedback: VecDeque<TaggedPacket>,
}

impl<P> KeyframeFeedback<P> {
    pub(super) fn new(next: P) -> Self {
        Self {
            next,
            feedback: VecDeque::new(),
        }
    }
}

#[interceptor]
impl<P: Interceptor> KeyframeFeedback<P> {
    #[overrides]
    fn handle_read(&mut self, msg: TaggedPacket) -> Result<(), Self::Error> {
        if let Packet::Rtcp(packets) = &msg.message {
            let feedback = packets
                .iter()
                .filter(|packet| super::asks_keyframe(std::slice::from_ref(*packet)))
                .cloned()
                .collect::<Vec<_>>();
            if !feedback.is_empty() {
                // The driver drains this queue each read cycle. Bound it even if
                // a remote sends feedback faster than the application can poll.
                if self.feedback.len() == 64 {
                    self.feedback.pop_front();
                }
                self.feedback.push_back(TaggedPacket {
                    now: msg.now,
                    transport: msg.transport,
                    message: Packet::Rtcp(feedback),
                });
            }
        }
        self.next.handle_read(msg)
    }

    #[overrides]
    fn poll_read(&mut self) -> Option<Self::Rout> {
        self.feedback.pop_front().or_else(|| self.next.poll_read())
    }

    #[overrides]
    fn close(&mut self) -> Result<(), Self::Error> {
        self.feedback.clear();
        self.next.close()
    }
}
