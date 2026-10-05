# Nixon Privacy Policy

*Last updated: 2026-10-04*

## Our Privacy-First Commitment

Nixon is built on the principle that your meeting data should remain private and under your control. This privacy policy explains how we handle data in our open-source meeting assistant.

## Data Processing Philosophy

### Local-First Processing
- **Meeting transcription**: Processed entirely on your device using local Whisper/Parakeet models
- **Audio recordings**: Never transmitted to external servers
- **Meeting content**: Remains on your device
- **AI summaries**: Generated locally (default) or through your chosen LLM provider

### Your Data Ownership
- You own all meeting data, transcripts, and recordings
- Data is stored locally on your device
- No vendor lock-in — export your data anytime
- Complete control over data retention and deletion

## Third-Party Services

### LLM Providers (Optional)
If you choose to use external LLM providers for summarization:
- **Local Ollama**: Processed entirely on your device (the default)
- **Anthropic Claude / OpenAI / Groq / OpenRouter / custom OpenAI-compatible endpoints**: Subject to that provider's privacy policy

### Update checks (on by default, optional)
- **nixonapp.com**: Nixon asks nixonapp.com (hosted on Cloudflare) for its latest release
  about every six hours and downloads new versions in the background. The request is a plain
  GET that carries the operating system (`darwin`) and app version you are running (for
  example `0.13.0`), plus what any download sends (your IP address and a generic user
  agent). It carries no account, device, or meeting information. The server counts update
  checks and downloads by event type, app version and platform with a timestamp, aggregated when read; Nixon's own
  systems store no IP address or user identifier. Cloudflare, as the host, processes request
  metadata under its own privacy policy. Copies of Nixon installed before the move to
  nixonapp.com still check github.com until they update. Turn update checks off under
  Settings > General > "Download updates automatically"; "Check for updates" in About then
  works on demand. Installing is always your click.

## Your Privacy Rights

### Data Control
- **Access**: View all data stored locally on your device
- **Export**: Export your data in standard formats
- **Delete**: Remove all data from your device

## Data Security

### Local Security
- Standard file system permissions protect your data
- No transmission of sensitive meeting data

### Open Source Transparency
- Full source code available for security review
- No hidden data collection or tracking

## Changes to This Policy

Material changes to this privacy policy will be reflected in this document in the project's GitHub repository and in release notes.

## Contact

For privacy-related questions or concerns:
- **GitHub Issues**: [Create an issue](https://github.com/sxates/nixon/issues)

## Open Source Commitment

As an open-source project under the MIT license, you can:
- Review the complete privacy implementation
- Modify data handling to meet your requirements
- Run entirely on your own device
- Contribute privacy improvements

---

*Nixon is a fork of [meetily](https://github.com/Zackriya-Solutions/meeting-minutes) (MIT).*
