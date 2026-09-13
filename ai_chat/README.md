# AI Chat

A dockable chat panel inside the editor. Pick a provider and model, type a prompt, and the reply streams in token by token. Requests run on a worker thread, so the editor never blocks while one is in flight.

**Open it:** the **AI Chat** panel, under the "AI" group in the panel list.

**Providers:** Ollama (local, no key), Claude, OpenAI, Grok, DeepSeek, Gemini, OpenRouter, and any OpenAI-compatible server such as LM Studio or llama.cpp. Everything but Ollama wants an API key, entered in the panel.

**Scope:** Editor. It never loads in an exported game, and deleting the library from `plugins/` removes the feature outright.
