Early preview of `dc-client`, an unofficial native Discord client written in Rust with GPUI. It shows your servers, channels and messages, updates live, and applies local-only cosmetics (name colors, display names, accent color, banner, avatar and avatar decoration from local image files). It cannot send messages yet, and there is no voice.

**Unverified.** Neither the connection to Discord nor the window's appearance has been checked by a person yet: it was built against mock transports, synthetic fixtures and a headless build machine. Follow [`docs/LOCAL_VERIFY.md`](https://github.com/staraepp/rust-discord/blob/main/docs/LOCAL_VERIFY.md) and expect things to break.

**Account risk.** Third-party clients violate Discord's Terms of Service and can get an account restricted or banned. Test with an account you can afford to lose.

## Install on Arch Linux

Download `dc-client-*-x86_64.pkg.tar.zst` and `SHA256SUMS` below, then:

```bash
sha256sum -c SHA256SUMS
```

```bash
sudo pacman -U dc-client-*-x86_64.pkg.tar.zst
```

Or build it yourself from `packaging/arch/` with `makepkg -si`. A working Vulkan driver is required.

## Use

The token is read from the `DC_TOKEN` environment variable only. Set it without leaving it in shell history, then start the app from the same terminal:

```bash
read -rs DC_TOKEN && export DC_TOKEN
```

```bash
dc-client
```

Cosmetics are configured in `~/.config/dc-client/cosmetics.json`; see [`docs/COSMETICS.md`](https://github.com/staraepp/rust-discord/blob/main/docs/COSMETICS.md).

The package also installs `dc-cli`, the headless tool, which is useful for debugging connection problems.

Remove with `sudo pacman -R dc-client`.
