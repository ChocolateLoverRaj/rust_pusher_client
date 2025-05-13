This is a [Pusher *Channels*](https://pusher.com/channels/) *compatible* (so you can use other Pusher-compatible online services or self-host (such as with [soketi](https://github.com/soketi/soketi))) *client* (you can subscribe to messages) library.

## Package Name
This package will be available as `pusher_client` on crates.io.

## Goals
- Implement functionality as it's requested. We'll start with basic subscribing to channels, and then we can do stuff like private, presense, and client events.
- Be maintained and if needed transition maintainers
- Not use any `unsafe`
- Be nice, easy, and Rusty to use
- Fit nice with graphical apps, such as apps that used [iced](https://iced.rs/)
- Be `async`
- Work on both web and native platforms. Currently it works on native only, but making it work on web is simply a matter of replacing the WebSocket library (help wanted). with a cross-platform one.
- Nicely handle all errors instead of panicking
