Early preview of `dc-client`, an unofficial native Discord client written in Rust with GPUI. It shows your servers, channels and messages, updates live, and applies local-only cosmetics (name colors, display names, accent color, banner, avatar and avatar decoration from local image files). The development branch now implements sending/replies, uploads, own-message edits/deletes, Markdown/images, emoji links, member pages, local settings and limited plugins. Voice packet/audio foundations are present, but working calls, DAVE, capture/Go Live and the rest of the blueprint are unfinished. Do not describe this checkpoint as the complete app.

**Unverified new features.** The owner verified an earlier connection/history build on Linux. The new interactions, visual behavior and device integration have only been checked headlessly against synthetic data. Follow [`docs/LOCAL_VERIFY.md`](https://github.com/staraepp/rust-discord/blob/main/docs/LOCAL_VERIFY.md) and expect things to break.

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

The token is read from the `DC_TOKEN` environment variable or the OS keyring. `dc-client --store-token` securely saves a session variable to the keyring; `--forget-token` removes it. No plaintext fallback is used. Set it without leaving it in shell history, then start the app from the same terminal:

```bash
read -rs DC_TOKEN && export DC_TOKEN
```

```bash
dc-client
```

Cosmetics are configured in `~/.config/dc-client/cosmetics.json`; see [`docs/COSMETICS.md`](https://github.com/staraepp/rust-discord/blob/main/docs/COSMETICS.md).

The package also installs `dc-cli`, the headless tool, which is useful for debugging connection problems.

Remove with `sudo pacman -R dc-client`.
