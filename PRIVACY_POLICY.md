# Nixon Privacy Policy

*Last updated: 2026-09-13*

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

## Usage Analytics

### What We Collect
Usage analytics is optional and off by default. When you choose to enable it, Nixon collects minimal, anonymized usage data:

**Application Usage:**
- Feature usage patterns (which tools you use most)
- Session duration and frequency
- Performance metrics (transcription success rates, error frequencies)
- UI interaction patterns (button clicks, navigation flows)

**Technical Metrics:**
- Application version and platform information
- Error logs and crash reports (anonymized)
- Performance benchmarks (processing times, resource usage)

### What We DON'T Collect
We never collect:
- ❌ Meeting content, transcripts, or recordings
- ❌ Personal information or identifiable data
- ❌ File names, meeting titles, or metadata
- ❌ Audio data or voice patterns
- ❌ Participant names or contact information
- ❌ LLM conversations or AI-generated content

### Analytics Implementation
- **Provider**: PostHog (privacy-focused analytics platform)
- **Default**: Off by default; analytics starts only after you enable it in settings
- **Anonymization**: All data linked to generated user IDs only — no personal identification
- **Data retention**: 12 months maximum, then automatically deleted
- **Encryption**: All data encrypted in transit using industry-standard protocols
- **Location**: Data processed in accordance with PostHog's privacy policy

## Third-Party Services

### LLM Providers (Optional)
If you choose to use external LLM providers for summarization:
- **Local Ollama**: Processed entirely on your device (the default)
- **Anthropic Claude / OpenAI / Groq / OpenRouter / custom OpenAI-compatible endpoints**: Subject to that provider's privacy policy

### Analytics Service (Optional)
- **PostHog**: Used for usage analytics when enabled
- **Data**: Only anonymized usage patterns, no meeting content
- **Control**: Completely optional, off by default, and user-controlled

## Your Privacy Rights

### Data Control
- **Access**: View all data stored locally on your device
- **Export**: Export your data in standard formats
- **Delete**: Remove all data from your device

### Analytics Transparency
- **Open source**: Full analytics implementation available for review in the source code
- **Opt-in**: New and existing installs have analytics disabled until you turn it on

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
