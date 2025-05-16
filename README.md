This is a [Pusher *Channels*](https://pusher.com/channels/) *compatible* (so you can use other Pusher-compatible online services or self-host (such as with [soketi](https://github.com/soketi/soketi))) *client* (you can subscribe to messages) library.

## Package Name
This package will be available as `pusher_client` on crates.io.

## Goals
- Implement functionality as it's requested. We'll start with basic subscribing to channels, and then we can do stuff like private, presence, and client events.
- Be maintained and if needed transition maintainers
- Not use any `unsafe`
- Be nice, easy, and Rusty to use
- Fit nice with graphical apps, such as apps that used [iced](https://iced.rs/)
- Be `async`
- Work on both web and native platforms. Currently it works on native only, but making it work on web is simply a matter of replacing the WebSocket library (help wanted). with a cross-platform one.
- Nicely handle all errors instead of panicking

## Status
- [x] Send pings to detect time since last message and if it gets disconnected
- [x] Plain channels (subscribe, receive messages, unsubscribe)
- [ ] Private channels
- [ ] Presence channels
- [ ] Encrypted channels
- [ ] Client events
- [x] Disconnect and reconnect gracefully (on an error, connection lost, or if app wants to disconnect)

## Demo
You can see a demo of using this crate in a Iced app that works on the web as well as native platforms. It's in `examples/app`. 

The live demo is at https://pusher-rs.netlify.app/, but it uses my Pusher app so you won't be able to create messages. Later I will add the ability to specify the app id so you can test it on your own Pusher compatible url.

To test the app on the web, run `trunk serve`. To test the app natively, run `cargo r`.
