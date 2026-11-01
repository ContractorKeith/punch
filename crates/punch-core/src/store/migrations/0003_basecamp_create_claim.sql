-- A short lease prevents concurrent mirrors from creating the same card.
ALTER TABLE basecamp_cards ADD COLUMN create_claim TEXT NULL;
