# Personal data

No personal data stands in this repository: not the names, handles, account ids, mail addresses, host
names, home paths, domains, devices or network details of the owner and the people they know; not a
room member's display name or id; not a line of an actual conversation; and not the name of an agent
or a product that uses the testbed. None of them stands in code, a comment, a document, a test fixture
or a commit message.

- **A value belongs to the installation that holds it.** An address, a host, a path, a port and a
  credential are read from a settings file or the environment; the code that reads one never carries
  its value.
- **An example is neutral.** A person is `tester` or `qa-admin`, an agent is `agent-a` or `agent-b`, a
  mailbox stands at `example.com`, and a home path names `user`.
- **An agent is named by its registration id,** never by the name it is given. The label a person sees
  stands in the deployment's settings.
- **A test's fixtures carry no real person, agent, room or message.** A conversation line a test reads
  is written for the test.

The `personal-data` quick check, which the declaration enables, reads every text file git tracks or
would track, a file that is not valid UTF-8 being passed over, for a mail address outside the
reserved example domains, a person's home path, a telephone number, a token and a private network
address, and names the file, the line and the kind of each finding, never the value (`docs/checks.md`,
「The quick checks」). The neutral forms an example writes are not findings: an address in
`example.com`, `example.net`, `example.org` or a domain ending `.example`, `.test`, `.invalid` or
`.localhost`, and a home folder named for a role, such as `user`, `tester`, `qa-admin`, `placeholder`
or `example`. The check carries the patterns alone; no list of the values it forbids stands in the
repository.
