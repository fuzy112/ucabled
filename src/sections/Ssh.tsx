import Section from "../components/Section";
import Reveal from "../components/Reveal";
import CopyButton from "../components/CopyButton";

const SSH = `# non-resident key (default): key handle lives in the .pub file
ssh-keygen -t ecdsa-sk -f ~/.ssh/id_ecdsa_sk

# resident (discoverable) key: stored as a passkey on the phone
ssh-keygen -t ecdsa-sk -O resident -f ~/.ssh/id_ecdsa_resident

# deploy + log in
ssh-copy-id -i ~/.ssh/id_ecdsa_sk.pub you@example.com
ssh -i ~/.ssh/id_ecdsa_sk you@example.com

# sign / verify a file, no ssh server involved
ssh-keygen -Y sign -f ~/.ssh/id_ecdsa_sk -n file ./file
ssh-keygen -Y verify -f allowed_signers -I "$USER" -n file -s ./file.sig < ./file`;

export default function Ssh() {
  return (
    <Section num="05" id="ssh" title="ssh security keys too">
      <Reveal>
        <p className="mb-6 max-w-3xl text-[13px] leading-relaxed" style={{ color: "var(--phos-55)" }}>
          openssh's fido <span style={{ color: "var(--phos)" }}>sk</span> key types talk to the virtual device directly —
          no browser needed. each operation pops the same QR window, and the phone asks for its
          passcode or biometric (user verification) before answering. works for login and for
          file signing.
        </p>
      </Reveal>
      <Reveal delay={80}>
        <div className="notch p-5 sm:p-6">
          <div className="mb-4 flex items-center justify-between">
            <span className="text-[10px] uppercase tracking-widest" style={{ color: "var(--phos-40)" }}>
              ~/.ssh — each command opens the qr window
            </span>
            <CopyButton text={SSH} />
          </div>
          <pre className="codeblock">{SSH}</pre>
        </div>
      </Reveal>
      <Reveal delay={140}>
        <p className="mt-5 text-[12px]" style={{ color: "var(--phos-40)" }}>
          note: ecdsa-sk only — phone passkey providers don't sign ed25519, and ssh-keygen -K
          (resident-key download) uses credential-management commands phones don't expose.
        </p>
      </Reveal>
    </Section>
  );
}
