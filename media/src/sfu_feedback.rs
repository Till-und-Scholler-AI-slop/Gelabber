//! Surface PLI/FIR to the SFU while keeping the default loss/report interceptors.
//! rtc 0.20.5's default chain consumes inbound RTCP before TrackLocal::poll;
//! its rtcp_processing_webrtc2webrtc integration test uses an outer interceptor
//! for this same purpose. Only keyframe feedback needs application forwarding.

use std::collections::VecDeque;

use rtc::interceptor::{Interceptor, Packet, StreamInfo, TaggedPacket, interceptor};
use rtc::rtcp::payload_feedbacks::full_intra_request::FullIntraRequest;
use rtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
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

#[cfg(test)]
mod tests {
    use super::*;
    use rtc::interceptor::NoopInterceptor;
    use rtc::rtcp::payload_feedbacks::full_intra_request::FirEntry;
    use rtc::sansio::Protocol;

    #[test]
    fn compound_target_queue_is_bounded_and_preserves_fir_sequence() {
        let mut interceptor = KeyframeFeedback::new(NoopInterceptor::new());
        let now = std::time::Instant::now();
        interceptor
            .handle_read(TaggedPacket {
                now,
                transport: Default::default(),
                message: Packet::Rtcp(vec![Box::new(FullIntraRequest {
                    sender_ssrc: 5,
                    media_ssrc: 0,
                    fir: (0..100)
                        .map(|ssrc| FirEntry {
                            ssrc,
                            sequence_number: ssrc as u8,
                        })
                        .collect(),
                })]),
            })
            .unwrap();
        assert_eq!(interceptor.feedback.len(), 64);
        for ssrc in 36..100 {
            let packet = interceptor.feedback.pop_front().unwrap();
            assert_eq!(packet.now, now);
            let Packet::Rtcp(packets) = packet.message else {
                panic!("RTCP feedback");
            };
            assert_eq!(packets.len(), 1);
            let fir = packets[0]
                .as_any()
                .downcast_ref::<FullIntraRequest>()
                .unwrap();
            assert_eq!(fir.sender_ssrc, 5);
            assert_eq!(
                fir.fir,
                vec![FirEntry {
                    ssrc,
                    sequence_number: ssrc as u8
                }]
            );
        }
    }
}

#[interceptor]
impl<P: Interceptor> KeyframeFeedback<P> {
    #[overrides]
    fn handle_read(&mut self, msg: TaggedPacket) -> Result<(), Self::Error> {
        if let Packet::Rtcp(packets) = &msg.message {
            for packet in packets {
                // The pinned endpoint routes only the first destination SSRC
                // in a TaggedPacket. Split compound packets AND FIR entries.
                if let Some(pli) = packet.as_any().downcast_ref::<PictureLossIndication>() {
                    self.push_feedback(&msg, Box::new(pli.clone()));
                } else if let Some(fir) = packet.as_any().downcast_ref::<FullIntraRequest>() {
                    for entry in &fir.fir {
                        self.push_feedback(
                            &msg,
                            Box::new(FullIntraRequest {
                                sender_ssrc: fir.sender_ssrc,
                                media_ssrc: fir.media_ssrc,
                                fir: vec![entry.clone()],
                            }),
                        );
                    }
                }
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

impl<P> KeyframeFeedback<P> {
    fn push_feedback(&mut self, msg: &TaggedPacket, packet: Box<dyn rtc::rtcp::Packet>) {
        // Bound by routed target, including a compound packet with many targets.
        if self.feedback.len() == 64 {
            self.feedback.pop_front();
        }
        self.feedback.push_back(TaggedPacket {
            now: msg.now,
            transport: msg.transport,
            message: Packet::Rtcp(vec![packet]),
        });
    }
}
