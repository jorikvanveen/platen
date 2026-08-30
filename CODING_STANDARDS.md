- Always use a DTO when returning data from a route. This may cause some duplicated code, but it is worthy tradeoff for making clear which types belong on the backend and which types will be sent to/from the frontend.
- Never edit generated files directly. Find a script or ask the user.
- If you need to generate a migration use the SeaORM CLI
- No panicking outside of tests, if something is unexpected return an error.
- If a comment is present, it should answer a "why" question. Never a "what" or "how" question.
- When mapping one struct to another, if possible prefer implementing `From` over writing a special function or doing the mapping inline.
  If the mapping requires more than one argument, prefer a helper function over a `From` impl on a tuple
- In frontend code, prefer <style> tags over a big app.css
