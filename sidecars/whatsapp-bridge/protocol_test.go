package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"strings"
	"testing"
	"time"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	"google.golang.org/protobuf/proto"
)

func encode(t *testing.T, value any) string {
	t.Helper()
	var buf bytes.Buffer
	out := newOutput(&buf)
	out.write(value)
	return buf.String()
}

func TestDecodeCommandSend(t *testing.T) {
	cmd, err := decodeCommand([]byte(`{"id":4,"cmd":"enviar","jid":"5511999999999@s.whatsapp.net","texto":"olá"}`))
	if err != nil {
		t.Fatalf("erro inesperado: %v", err)
	}
	if cmd.ID != 4 || cmd.Cmd != CmdSend || cmd.JID != "5511999999999@s.whatsapp.net" || cmd.Text != "olá" {
		t.Fatalf("comando decodificado errado: %+v", cmd)
	}
}

func TestDecodeCommandListMessages(t *testing.T) {
	cmd, err := decodeCommand([]byte(`{"id":3,"cmd":"listar_mensagens","jid":"123@g.us","limite":50}`))
	if err != nil {
		t.Fatalf("erro inesperado: %v", err)
	}
	if cmd.ID != 3 || cmd.Cmd != CmdListMessages || cmd.Limit != 50 || cmd.JID != "123@g.us" {
		t.Fatalf("comando decodificado errado: %+v", cmd)
	}
}

func TestDecodeCommandZeroID(t *testing.T) {
	cmd, err := decodeCommand([]byte(`{"id":0,"cmd":"login"}`))
	if err != nil {
		t.Fatalf("erro inesperado: %v", err)
	}
	if cmd.ID != 0 || cmd.Cmd != CmdLogin {
		t.Fatalf("comando decodificado errado: %+v", cmd)
	}
}

func TestDecodeCommandErrors(t *testing.T) {
	cases := map[string]string{
		"json invalido": `{"id":1,"cmd":`,
		"sem id":        `{"cmd":"login"}`,
		"sem cmd":       `{"id":7}`,
		"id texto":      `{"id":"1","cmd":"login"}`,
	}
	for name, line := range cases {
		if _, err := decodeCommand([]byte(line)); err == nil {
			t.Errorf("%s: esperava erro", name)
		}
	}
	if _, err := decodeCommand([]byte("   ")); !errors.Is(err, errEmptyCommand) {
		t.Errorf("linha vazia deveria gerar errEmptyCommand, veio %v", err)
	}
}

func TestDecodeCommandMissingCmdKeepsID(t *testing.T) {
	cmd, err := decodeCommand([]byte(`{"id":7}`))
	if err == nil {
		t.Fatal("esperava erro")
	}
	if cmd.ID != 7 {
		t.Fatalf("id deveria ser preservado, veio %d", cmd.ID)
	}
}

func TestOKResponseFormat(t *testing.T) {
	got := encode(t, okResponse(1, nil))
	if got != "{\"id\":1,\"ok\":true,\"dados\":null}\n" {
		t.Fatalf("formato inesperado: %q", got)
	}
	got = encode(t, okResponse(3, []Message{}))
	if got != "{\"id\":3,\"ok\":true,\"dados\":[]}\n" {
		t.Fatalf("formato inesperado: %q", got)
	}
}

func TestErrorResponseFormat(t *testing.T) {
	got := encode(t, errorResponse(2, "falhou <aqui>"))
	if got != "{\"id\":2,\"ok\":false,\"erro\":\"falhou <aqui>\"}\n" {
		t.Fatalf("formato inesperado: %q", got)
	}
}

func TestEventFormats(t *testing.T) {
	cases := []struct {
		event Event
		want  string
	}{
		{qrEvent("2@abc"), `{"evento":"qr","codigo":"2@abc"}`},
		{connectedEvent("5511@s.whatsapp.net"), `{"evento":"conectado","jid":"5511@s.whatsapp.net"}`},
		{disconnectedEvent(ReasonSessionEnded), `{"evento":"desconectado","motivo":"sessao_encerrada"}`},
		{disconnectedEvent(ReasonConnectionLost), `{"evento":"desconectado","motivo":"conexao_perdida"}`},
		{errorEvent("algo deu errado"), `{"evento":"erro","mensagem":"algo deu errado"}`},
	}
	for _, c := range cases {
		got := strings.TrimSuffix(encode(t, c.event), "\n")
		if got != c.want {
			t.Errorf("esperava %s, veio %s", c.want, got)
		}
	}
}

func TestMessageEventFormat(t *testing.T) {
	msg := Message{ID: "ABC", Chat: "1@s.whatsapp.net", From: "1@s.whatsapp.net", Text: "oi", At: 10, Kind: KindText}
	live := strings.TrimSuffix(encode(t, messageEvent(msg, false)), "\n")
	wantLive := `{"evento":"mensagem_recebida","mensagem":{"id":"ABC","jid":"1@s.whatsapp.net","de":"1@s.whatsapp.net","nome_de":"","texto":"oi","minha":false,"em":10,"tipo":"texto"}}`
	if live != wantLive {
		t.Fatalf("esperava %s, veio %s", wantLive, live)
	}
	history := strings.TrimSuffix(encode(t, messageEvent(msg, true)), "\n")
	wantHistory := `{"evento":"mensagem_recebida","mensagem":{"id":"ABC","jid":"1@s.whatsapp.net","de":"1@s.whatsapp.net","nome_de":"","texto":"oi","minha":false,"em":10,"tipo":"texto"},"historico":true}`
	if history != wantHistory {
		t.Fatalf("esperava %s, veio %s", wantHistory, history)
	}
}

func TestOutputOneLinePerMessage(t *testing.T) {
	var buf bytes.Buffer
	out := newOutput(&buf)
	out.write(qrEvent("a"))
	out.write(okResponse(1, map[string]string{"texto": "linha1\nlinha2"}))
	lines := strings.Split(strings.TrimSuffix(buf.String(), "\n"), "\n")
	if len(lines) != 2 {
		t.Fatalf("esperava 2 linhas, veio %d: %q", len(lines), buf.String())
	}
	for _, line := range lines {
		var decoded map[string]any
		if err := json.Unmarshal([]byte(line), &decoded); err != nil {
			t.Fatalf("linha não é JSON válido: %q", line)
		}
	}
}

func TestClassifyMessage(t *testing.T) {
	cases := []struct {
		name string
		msg  *waE2E.Message
		kind string
		text string
	}{
		{"nil", nil, KindOther, ""},
		{"conversation", &waE2E.Message{Conversation: proto.String("oi")}, KindText, "oi"},
		{"extended", &waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{Text: proto.String("link http://x")}}, KindText, "link http://x"},
		{"imagem com legenda", &waE2E.Message{ImageMessage: &waE2E.ImageMessage{Caption: proto.String("foto")}}, KindImage, "foto"},
		{"imagem sem legenda", &waE2E.Message{ImageMessage: &waE2E.ImageMessage{}}, KindImage, ""},
		{"video", &waE2E.Message{VideoMessage: &waE2E.VideoMessage{Caption: proto.String("vid")}}, KindVideo, "vid"},
		{"audio", &waE2E.Message{AudioMessage: &waE2E.AudioMessage{}}, KindAudio, ""},
		{"documento", &waE2E.Message{DocumentMessage: &waE2E.DocumentMessage{Caption: proto.String("doc")}}, KindDocument, "doc"},
		{"documento com legenda", &waE2E.Message{DocumentWithCaptionMessage: &waE2E.FutureProofMessage{Message: &waE2E.Message{DocumentMessage: &waE2E.DocumentMessage{Caption: proto.String("leg")}}}}, KindDocument, "leg"},
		{"figurinha", &waE2E.Message{StickerMessage: &waE2E.StickerMessage{}}, KindSticker, ""},
		{"outro", &waE2E.Message{ReactionMessage: &waE2E.ReactionMessage{Text: proto.String("x")}}, KindOther, ""},
	}
	for _, c := range cases {
		kind, text := classifyMessage(c.msg)
		if kind != c.kind || text != c.text {
			t.Errorf("%s: esperava (%s, %q), veio (%s, %q)", c.name, c.kind, c.text, kind, text)
		}
	}
}

func TestMessageFromEvent(t *testing.T) {
	chat := types.NewJID("123456", types.GroupServer)
	sender := types.NewJID("5511999999999", types.DefaultUserServer)
	at := time.Unix(1700000000, 0)
	evt := &events.Message{
		Info: types.MessageInfo{
			MessageSource: types.MessageSource{Chat: chat, Sender: sender, IsFromMe: false, IsGroup: true},
			ID:            "MSG1",
			PushName:      "Fulano",
			Timestamp:     at,
		},
		Message: &waE2E.Message{ImageMessage: &waE2E.ImageMessage{Caption: proto.String("veja")}},
	}
	got := messageFromEvent(evt)
	want := Message{
		ID:       "MSG1",
		Chat:     "123456@g.us",
		From:     "5511999999999@s.whatsapp.net",
		FromName: "Fulano",
		Text:     "veja",
		Mine:     false,
		At:       1700000000,
		Kind:     KindImage,
	}
	if got != want {
		t.Fatalf("esperava %+v, veio %+v", want, got)
	}
}

func TestIsRelevantMessage(t *testing.T) {
	user := types.NewJID("5511", types.DefaultUserServer)
	base := types.MessageInfo{MessageSource: types.MessageSource{Chat: user, Sender: user}}
	text := &events.Message{Info: base, Message: &waE2E.Message{Conversation: proto.String("oi")}}
	if !isRelevantMessage(text) {
		t.Error("mensagem de texto deveria ser relevante")
	}
	reaction := &events.Message{Info: base, Message: &waE2E.Message{ReactionMessage: &waE2E.ReactionMessage{Text: proto.String("x")}}}
	if isRelevantMessage(reaction) {
		t.Error("reação não deveria ser relevante")
	}
	protocol := &events.Message{Info: base, Message: &waE2E.Message{ProtocolMessage: &waE2E.ProtocolMessage{}}}
	if isRelevantMessage(protocol) {
		t.Error("mensagem de protocolo não deveria ser relevante")
	}
	status := &events.Message{
		Info:    types.MessageInfo{MessageSource: types.MessageSource{Chat: types.StatusBroadcastJID, Sender: user}},
		Message: &waE2E.Message{Conversation: proto.String("status")},
	}
	if isRelevantMessage(status) {
		t.Error("status não deveria ser relevante")
	}
	if isRelevantMessage(&events.Message{Info: base}) {
		t.Error("mensagem vazia não deveria ser relevante")
	}
}

func TestSentMessage(t *testing.T) {
	chat := types.NewJID("5521888888888", types.DefaultUserServer)
	own := types.NewJID("5511999999999", types.DefaultUserServer)
	got := sentMessage("OUT1", chat, own, "olá", time.Unix(1700000100, 0))
	want := Message{
		ID:   "OUT1",
		Chat: "5521888888888@s.whatsapp.net",
		From: "5511999999999@s.whatsapp.net",
		Text: "olá",
		Mine: true,
		At:   1700000100,
		Kind: KindText,
	}
	if got != want {
		t.Fatalf("esperava %+v, veio %+v", want, got)
	}
}

func TestBuildConversations(t *testing.T) {
	contacts := map[types.JID]types.ContactInfo{
		types.NewJID("5511000000001", types.DefaultUserServer): {Found: true, FullName: "Bruno Silva", PushName: "Bru"},
		types.NewJID("5511000000002", types.DefaultUserServer): {Found: true, PushName: "Ana"},
		types.NewJID("5511000000003", types.DefaultUserServer): {Found: false},
		types.NewJID("999", types.GroupServer):                 {Found: true, FullName: "Ignorado"},
	}
	groups := []*types.GroupInfo{
		{JID: types.NewJID("120363000", types.GroupServer), GroupName: types.GroupName{Name: "Família"}},
		nil,
	}
	got := buildConversations(contacts, groups)
	want := []Conversation{
		{JID: "5511000000003@s.whatsapp.net", Name: "5511000000003", Group: false},
		{JID: "5511000000002@s.whatsapp.net", Name: "Ana", Group: false},
		{JID: "5511000000001@s.whatsapp.net", Name: "Bruno Silva", Group: false},
		{JID: "120363000@g.us", Name: "Família", Group: true},
	}
	if len(got) != len(want) {
		t.Fatalf("esperava %d conversas, veio %d: %+v", len(want), len(got), got)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Errorf("posição %d: esperava %+v, veio %+v", i, want[i], got[i])
		}
	}
	encoded := strings.TrimSuffix(encode(t, got[3]), "\n")
	if encoded != `{"jid":"120363000@g.us","nome":"Família","grupo":true}` {
		t.Fatalf("formato inesperado: %s", encoded)
	}
}

func TestContactNameFallbacks(t *testing.T) {
	jid := types.NewJID("5511777", types.DefaultUserServer)
	if got := contactName(jid, types.ContactInfo{FirstName: "Zé"}); got != "Zé" {
		t.Errorf("esperava Zé, veio %s", got)
	}
	if got := contactName(jid, types.ContactInfo{BusinessName: "Loja"}); got != "Loja" {
		t.Errorf("esperava Loja, veio %s", got)
	}
	if got := contactName(jid, types.ContactInfo{PushName: "  "}); got != "5511777" {
		t.Errorf("esperava o número, veio %s", got)
	}
}

func TestSessionDSN(t *testing.T) {
	dsn := sessionDSN("dados")
	if !strings.Contains(dsn, sessionFileName) || !strings.Contains(dsn, "_pragma=foreign_keys(1)") {
		t.Fatalf("dsn inesperado: %s", dsn)
	}
}
