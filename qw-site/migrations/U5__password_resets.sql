CREATE TABLE password_reset (
  secret VARCHAR(32) PRIMARY KEY,
  account_id INTEGER NOT NULL,
  time_sent BIGINT NOT NULL,

  FOREIGN KEY(account_id) REFERENCES account(id)
);
