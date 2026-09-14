# Why small languages

Every construct a language adds is another form its checker must learn to
distrust. C++, to take the extreme case, has roughly twenty forms of
initialization. Soil has one. That austerity is the entry fee for a
language meant to be written by machines rather than people.

Hallucination, in an agent-written codebase, means emitting plausible code
that nobody asked for. An agent hallucinates in proportion to the choices
its language offers it. Fewer forms, then, means fewer places to be wrong.
