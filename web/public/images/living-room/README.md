# Living Room artwork

`community-banner.png` is the mountain sunset artwork generated for the approved
Gelabber Living Room concept on 2026-10-01 using OpenAI Image Gen. It is decorative
community artwork, not an image of a member or a live screen-share preview.

The sign-in pages use `community-banner.webp`, the same picture at the same size
(`cwebp -q 80 -m 6 -metadata none`): it is drawn at 18 % opacity, and the PNG
weighs 2 MB. The PNG stays for the native desktop app, which loads it from the
server (`desktop/ui/src/screens/auth/layout.rs`).

The original local generation is
`exec-854e6f09-b467-47e4-a051-63700a874fca.png` from session
`01a0f417-6124-7660-a90f-81039a041692`.

Member images continue to come from actual account profiles, with initials when
no image is set. Stream tiles use actual MediaStreams from Gelabber's own SFU.
