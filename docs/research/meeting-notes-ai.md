# Meeting notes: local or cloud AI?

Date: 2026-11-06. Question from the user: "maybe local running PC is good? Which is the cheapest way with the best output?"

## Your PC (measured)
- GPU: NVIDIA GeForce RTX 4060 Ti, 8 GB VRAM
- CPU: Intel Core i5-13600K (14 cores, 20 threads)
- RAM: 32 GB

This PC can run both models in this note locally, one after the other: speech to text during the meeting, and the summary afterwards.

## Speech to text
| Option | Indonesian | Cost | Notes |
|---|---|---|---|
| **Whisper large-v3-turbo, local** (whisper.cpp with CUDA, Rust binding `whisper-rs`) | About 6.5% WER on FLEURS Indonesian (a vendor's own run on read speech, not independent) | Free | Same model as Groq. Fits easily in 8 GB. Real meetings will score worse than read speech. |
| Whisper large-v3-turbo on Groq | Same model | $0.04 per hour of audio (Groq's pricing page), billed at least 10 s per request | Fallback for PCs without a GPU, such as a laptop. 20 hours of meetings a month is about $0.80. |
| Deepgram Nova-3 | Supported since Oct 2025 | About $0.0043 to $0.0052 per minute (third-party listings disagree), so roughly $0.26 to $0.31 per hour | Good streaming, but 6 to 8 times Groq's price. |
| OpenAI gpt-4o-mini-transcribe | Supported | About $0.006 per minute ($0.36 per hour) | No reason to pick it over Groq here. |
| NVIDIA Parakeet TDT 0.6B v3 | **Not supported** (NVIDIA lists 25 European languages) | Free | The meetings spec named Parakeet. It is dropped for Indonesian. |

## Summary (decisions and action items)
| Option | Cost | Notes |
|---|---|---|
| **Local 8B model** through llama.cpp or Ollama, 4-bit, about 5 GB | Free | A 2025 UIN Sunan Kalijaga thesis on Indonesian summarisation found the Indonesian-tuned **Sahabat-AI** Llama3 8B the most balanced and Sahabat-AI Gemma2 9B the best on coverage. SEA-LION scored lower. The thesis is unreviewed, used 8-bit models, and tested news, not meetings. |
| A small cloud model | Well under one US cent per one-hour meeting (estimate: an hour of talk is about 15k tokens of text) | Better at clean action items than a local 8B. Only text is sent, never audio. |

## Superseded first recommendation (local first)
The first version of this note recommended local Whisper plus a local 8B model. The user rejected local models ("no local model pls"), so it is replaced by the Groq-only decision below.

## Decision (2026-11-06): Groq first, for internal use
The user's case: internal use only, client calls in English and team calls in Indonesian, 30 to 90 min each.
- **Default engine: Groq Whisper large-v3-turbo**, one engine for both languages. The language is picked per meeting in "Take notes ⌄": English, Bahasa Indonesia, or Auto (decided in the first 30 s). It is remembered per calendar or app.
- **Why Groq over local:** the output is the same model, so the quality is the same. It costs no GPU or RAM on the PC, which also makes the MacBook M1 8 GB workable later. Its free tier fits this use.
- **Free tier fit** (Groq docs: 7,200 audio seconds per hour, 28,800 per day, 2,000 requests per day):
  - Two streams (Me and Them) in real time use up to 2 h of audio per clock hour, which is exactly the cap. So only speech is sent: a voice-activity cut drops silence, which in a normal call should be well under the cap. If it still hits the cap, the mic and meeting audio are mixed into one stream.
  - The daily cap is 8 h of audio, about 3 to 5 meetings of 90 min a day once silence is cut.
  - Chunks are cut at pauses, about 5 to 15 s long. Each request is billed at least 10 s, so 2,000 requests a day is plenty.
- **Live feel:** text appears per chunk, a few seconds behind speech, not word by word like Deepgram. That is fine for notes.
- **No local models (user, 2026-11-06).** When offline or over the cap, the audio waits on the PC (encrypted) and is sent when possible. The note then says "Transcript coming when you're back online". Deepgram is not needed.
- **On trial.** If the real-speech test fails (Indonesian team talk, English client calls), the meeting-notes feature is turned off rather than patched with local models.
- **Notes model:** `qwen/qwen3.8-27b` on Groq. It was the most accurate in the test. Headings come from our template, not the model.

## Tested on the user's Groq key (2026-11-06)
- **Speech to text:** 25 s of English (a Windows TTS voice) came back in 1.9 s, round trip included. A "Glossary:" prompt fixed a name ("Demas" became "Dimas"). Free limits from the response headers: 7,200 audio seconds per hour and 2,000 requests per day.
- **Summary models on the account:** `openai/gpt-oss-120b`, `openai/gpt-oss-20b` and `qwen/qwen3.8-27b`, each with a 131k context. Free limits: **8,000 tokens per minute** and 1,000 requests per day.
- **Indonesian team-meeting sample** (9 lines, Jumat 6 Nov):
  - `qwen3.8-27b` (0.6 s): accurate, no invented facts. It dropped the section headings.
  - `gpt-oss-120b` (1.3 s): the best structure, but **wrong dates**: "Rabu, 9 Nov" (should be 11 Nov) and "Jumat, 8 Nov" (should be 13 Nov).
- **Design rules from this:**
  - The model never converts days to dates. It quotes the spoken deadline ("sebelum Rabu"), and `crates/core`'s Indonesian/English date parser resolves it against the meeting date.
  - A 90 min meeting (about 20 to 30k tokens) does not fit 8k tokens per minute in one request. So the summary is **rolling**: every 10 min of transcript is condensed while the meeting runs, and at Stop only one small merge runs. Notes are ready in seconds, and it also enables "what did I just miss".
- **How Granola does notes** (from `granolarea/research/03-notes-and-ai-generation.md`):
  - All LLM calls go through Granola's own server. Prompts are named by slug and the prompt text lives on the server.
  - One summary slug is `meeting-summary-consolidated-sonnet-4-6` (likely Claude Sonnet).
  - Chat offers Claude, GPT, Gemini, Grok, Kimi, GLM, DeepSeek and Qwen.
  - Inputs: transcript with speaker names, your typed notes, attendees and companies from the calendar, and the template sections.
  - The output streams into the editor with each AI bullet marked as AI. An edited bullet becomes yours.
  - Action items are a separate step. A small "did the meeting end" prompt reads the last 100 words for a goodbye.

## Before we commit (honest unknowns)
- **Mixed Indonesian and English speech** is the real risk. Whisper picks one language per 30 s window and can translate instead of transcribe. Test with 2 or 3 of your real meetings: auto-detect, forced `id`, and an Indonesian fine-tune.
- **Live transcript speed** on the 4060 Ti is my estimate (turbo should run well above real time). I'll measure it in the prototype.
- **Prices** come from Groq's page and third-party listings, as of November 2026. Check them again before shipping.

## Sources
- Groq pricing and the Whisper turbo launch: https://groq.com/pricing, https://groq.com/?p=5392
- Indonesian WER (turbo, vendor run): https://whispernotes.app/blog/qwen3-asr-vs-whisper. Indonesian fine-tunes: https://huggingface.co/cahya/whisper-medium-id
- Deepgram Nova-3 adds Indonesian: https://deepgram.com/learn/deepgram-expands-nova-3-with-italian-turkish-norwegian-and-indonesian-support. Price listing: https://openrouter.ai/deepgram/nova-3
- Parakeet languages: https://catalog.ngc.nvidia.com/orgs/nim/nvidia/containers/parakeet-0.6b-tdt/-
- Indonesian summarisation thesis: https://digilib.uin-suka.ac.id/id/eprint/72937/
- gpt-4o-mini-transcribe price: https://www.getmaxim.ai/bifrost/llm-cost-calculator/provider/openai/model/gpt-4o-mini-transcribe.md
