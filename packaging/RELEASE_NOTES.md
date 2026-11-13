Pre-release of `dc-cli`, the headless dev tool from milestone M0. There is no GUI, voice or plugin support yet.

**Unverified against live Discord.** This build has only been tested against mock transports and synthetic fixtures. Follow [`docs/LOCAL_VERIFY.md`](https://github.com/staraepp/rust-discord/blob/main/docs/LOCAL_VERIFY.md) and expect things to break.

**Account risk.** Third-party clients violate Discord's Terms of Service and can get an account restricted or banned. Test with an account you can afford to lose.

## Install on Arch Linux

Download `dc-cli-*-x86_64.pkg.tar.zst` and `SHA256SUMS` below, then:

```bash
sha256sum -c SHA256SUMS
```

```bash
sudo pacman -U dc-cli-*-x86_64.pkg.tar.zst
```

Or build it yourself from `packaging/arch/` with `makepkg -si`.

## Use

The token is read from the `DC_TOKEN` environment variable only. Set it without leaving it in shell history:

```bash
read -rs DC_TOKEN && export DC_TOKEN
```

```bash
dc-cli guilds
```

Remove with `sudo pacman -R dc-cli`.
