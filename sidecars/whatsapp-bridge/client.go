package main

import (
	"context"
	"errors"
	"fmt"
	"log"
	"mime"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/store"
	"go.mau.fi/whatsmeow/store/sqlstore"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	waLog "go.mau.fi/whatsmeow/util/log"
	"google.golang.org/protobuf/proto"
	_ "modernc.org/sqlite"
)

const sessionFileName = "whatsapp-sessao.db"

var errNotConnected = errors.New("o WhatsApp não está conectado")

type stderrLogger struct {
	module string
	debug  bool
	out    *log.Logger
}

func newStderrLogger(module string, debug bool) waLog.Logger {
	return &stderrLogger{module: module, debug: debug, out: log.New(os.Stderr, "", log.LstdFlags)}
}

func (l *stderrLogger) write(level, msg string, args ...any) {
	l.out.Printf("[%s %s] %s", l.module, level, fmt.Sprintf(msg, args...))
}

func (l *stderrLogger) Warnf(msg string, args ...any)  { l.write("WARN", msg, args...) }
func (l *stderrLogger) Errorf(msg string, args ...any) { l.write("ERROR", msg, args...) }
func (l *stderrLogger) Infof(msg string, args ...any)  { l.write("INFO", msg, args...) }

func (l *stderrLogger) Debugf(msg string, args ...any) {
	if l.debug {
		l.write("DEBUG", msg, args...)
	}
}

func (l *stderrLogger) Sub(module string) waLog.Logger {
	return &stderrLogger{module: l.module + "/" + module, debug: l.debug, out: l.out}
}

type bridge struct {
	ctx       context.Context
	dataDir   string
	container *sqlstore.Container
	out       *output
	log       waLog.Logger
	mu        sync.Mutex
	client    atomic.Pointer[whatsmeow.Client]
}

func sessionDSN(dataDir string) string {
	path := filepath.Join(dataDir, sessionFileName)
	return path + "?_pragma=foreign_keys(1)&_pragma=busy_timeout(10000)&_pragma=journal_mode(WAL)"
}

func newBridge(ctx context.Context, dataDir string, out *output) (*bridge, error) {
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		return nil, fmt.Errorf("não foi possível criar a pasta de dados: %w", err)
	}
	logger := newStderrLogger("whatsapp", os.Getenv("WHATSAPP_BRIDGE_DEBUG") != "")
	container, err := sqlstore.New(ctx, "sqlite", sessionDSN(dataDir), logger.Sub("Banco"))
	if err != nil {
		return nil, fmt.Errorf("não foi possível abrir a sessão do WhatsApp: %w", err)
	}
	device, err := container.GetFirstDevice(ctx)
	if err != nil {
		return nil, fmt.Errorf("não foi possível ler a sessão do WhatsApp: %w", err)
	}
	b := &bridge{ctx: ctx, dataDir: dataDir, container: container, out: out, log: logger}
	b.client.Store(b.newClient(device))
	return b, nil
}

func (b *bridge) newClient(device *store.Device) *whatsmeow.Client {
	client := whatsmeow.NewClient(device, b.log.Sub("Cliente"))
	client.AddEventHandler(func(evt any) {
		b.handleEvent(client, evt)
	})
	return client
}

func (b *bridge) current() *whatsmeow.Client {
	return b.client.Load()
}

func (b *bridge) handle(cmd Command) {
	switch cmd.Cmd {
	case CmdLogin:
		b.out.write(okResponse(cmd.ID, nil))
		go func() {
			if err := b.login(); err != nil {
				b.log.Errorf("falha ao conectar: %v", err)
				b.out.write(errorEvent("Não foi possível conectar ao WhatsApp: " + err.Error()))
			}
		}()
	case CmdListConversations:
		conversations, err := b.listConversations()
		if err != nil {
			b.out.write(errorResponse(cmd.ID, "Não foi possível listar as conversas: "+err.Error()))
			return
		}
		b.out.write(okResponse(cmd.ID, conversations))
	case CmdListMessages:
		b.out.write(okResponse(cmd.ID, []Message{}))
	case CmdChatInfo:
		infos, err := b.chatInfo(cmd.JIDs)
		if err != nil {
			b.out.write(errorResponse(cmd.ID, "Não foi possível consultar as conversas: "+err.Error()))
			return
		}
		b.out.write(okResponse(cmd.ID, infos))
	case CmdDownloadMedia:
		path, err := b.downloadImage(cmd.MessageID, cmd.Media)
		if err != nil {
			b.out.write(errorResponse(cmd.ID, "Não foi possível baixar a imagem: "+err.Error()))
			return
		}
		b.out.write(okResponse(cmd.ID, DownloadResult{Path: path}))
	case CmdSendFile:
		msg, err := b.sendFile(cmd.JID, cmd.Path, cmd.Text)
		if err != nil {
			b.out.write(errorResponse(cmd.ID, "Não foi possível enviar o arquivo: "+err.Error()))
			return
		}
		b.out.write(okResponse(cmd.ID, msg))
	case CmdParticipants:
		participants, err := b.participants(cmd.JID)
		if err != nil {
			b.out.write(errorResponse(cmd.ID, "Não foi possível listar os participantes: "+err.Error()))
			return
		}
		b.out.write(okResponse(cmd.ID, participants))
	case CmdSend:
		msg, err := b.send(cmd.JID, cmd.Text, cmd.JIDs)
		if err != nil {
			b.out.write(errorResponse(cmd.ID, "Não foi possível enviar a mensagem: "+err.Error()))
			return
		}
		b.out.write(okResponse(cmd.ID, msg))
	default:
		b.out.write(errorResponse(cmd.ID, "Comando desconhecido: "+cmd.Cmd))
	}
}

func (b *bridge) login() error {
	b.mu.Lock()
	defer b.mu.Unlock()
	client := b.current()
	if client.IsConnected() {
		if client.IsLoggedIn() && client.Store.ID != nil {
			b.out.write(connectedEvent(client.Store.ID.ToNonAD().String()))
		}
		return nil
	}
	if client.Store.ID == nil {
		qr, err := client.GetQRChannel(b.ctx)
		if err != nil {
			return err
		}
		go b.watchQR(qr)
	}
	return client.Connect()
}

func (b *bridge) watchQR(qr <-chan whatsmeow.QRChannelItem) {
	for item := range qr {
		switch item.Event {
		case whatsmeow.QRChannelEventCode:
			b.out.write(qrEvent(item.Code))
		case whatsmeow.QRChannelSuccess.Event:
			b.log.Infof("pareamento concluído")
		case whatsmeow.QRChannelTimeout.Event:
			b.out.write(errorEvent("O código QR expirou. Tente entrar de novo."))
		case whatsmeow.QRChannelEventError:
			detail := "erro desconhecido"
			if item.Error != nil {
				detail = item.Error.Error()
			}
			b.out.write(errorEvent("Falha no pareamento: " + detail))
		case whatsmeow.QRChannelClientOutdated.Event:
			b.out.write(errorEvent("Esta versão do cliente do WhatsApp está desatualizada."))
		default:
			if strings.HasPrefix(item.Event, "passkey") {
				b.log.Warnf("evento de pareamento não suportado: %s", item.Event)
				continue
			}
			b.out.write(errorEvent("Falha no pareamento: " + item.Event))
		}
	}
}

func (b *bridge) listConversations() ([]Conversation, error) {
	client := b.current()
	if !client.IsLoggedIn() {
		return nil, errNotConnected
	}
	ctx, cancel := context.WithTimeout(b.ctx, 30*time.Second)
	defer cancel()
	contacts, err := client.Store.Contacts.GetAllContacts(ctx)
	if err != nil {
		return nil, err
	}
	groups, err := client.GetJoinedGroups(ctx)
	if err != nil {
		b.log.Warnf("falha ao obter grupos: %v", err)
		groups = nil
	}
	return buildConversations(contacts, groups), nil
}

func (b *bridge) resolve(ctx context.Context, client *whatsmeow.Client, jid types.JID) types.JID {
	jid = jid.ToNonAD()
	if jid.Server != types.HiddenUserServer {
		return jid
	}
	pn, err := client.Store.LIDs.GetPNForLID(ctx, jid)
	if err != nil || pn.IsEmpty() {
		return jid
	}
	return pn.ToNonAD()
}

func (b *bridge) savedName(ctx context.Context, client *whatsmeow.Client, jid types.JID) string {
	if jid.Server != types.DefaultUserServer {
		return ""
	}
	info, err := client.Store.Contacts.GetContact(ctx, jid)
	if err != nil || !info.Found {
		return ""
	}
	return savedName(info)
}

func (b *bridge) archived(ctx context.Context, client *whatsmeow.Client, jids ...types.JID) bool {
	for _, jid := range jids {
		settings, err := client.Store.ChatSettings.GetChatSettings(ctx, jid)
		if err == nil && settings.Found && settings.Archived {
			return true
		}
	}
	return false
}

func (b *bridge) chatInfo(jids []string) ([]ChatInfo, error) {
	client := b.current()
	if !client.IsLoggedIn() {
		return nil, errNotConnected
	}
	ctx, cancel := context.WithTimeout(b.ctx, 60*time.Second)
	defer cancel()
	result := make([]ChatInfo, 0, len(jids))
	for _, raw := range jids {
		jid, err := types.ParseJID(raw)
		if err != nil {
			continue
		}
		jid = jid.ToNonAD()
		resolved := b.resolve(ctx, client, jid)
		info := ChatInfo{JID: raw, Name: b.savedName(ctx, client, resolved)}
		if resolved != jid {
			info.Phone = resolved.String()
		}
		info.Archived = b.archived(ctx, client, jid, resolved)
		result = append(result, info)
	}
	return result, nil
}

func (b *bridge) downloadImage(id, media string) (string, error) {
	img, err := decodeImage(media)
	if err != nil {
		return "", err
	}
	dir := filepath.Join(b.dataDir, "midia")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return "", err
	}
	path := filepath.Join(dir, mediaFileName(id, img.GetMimetype()))
	if info, err := os.Stat(path); err == nil && info.Size() > 0 {
		return path, nil
	}
	client := b.current()
	if !client.IsLoggedIn() {
		return "", errNotConnected
	}
	ctx, cancel := context.WithTimeout(b.ctx, 2*time.Minute)
	defer cancel()
	data, err := client.Download(ctx, img)
	if err != nil {
		if errors.Is(err, whatsmeow.ErrMediaDownloadFailedWith404) || errors.Is(err, whatsmeow.ErrMediaDownloadFailedWith410) {
			return "", errors.New("a imagem não está mais disponível no servidor do WhatsApp")
		}
		return "", err
	}
	if err := os.WriteFile(path, data, 0o644); err != nil {
		return "", err
	}
	return path, nil
}

func (b *bridge) enrich(client *whatsmeow.Client, evt *events.Message, msg *Message) {
	ctx, cancel := context.WithTimeout(b.ctx, 10*time.Second)
	defer cancel()
	info := evt.Info
	chat := b.resolve(ctx, client, info.Chat)
	if chat.Server == types.HiddenUserServer && !info.IsGroup {
		alt := info.SenderAlt
		if info.IsFromMe {
			alt = info.RecipientAlt
		}
		if alt.Server == types.DefaultUserServer {
			chat = alt.ToNonAD()
		}
	}
	msg.Chat = chat.String()
	sender := b.resolve(ctx, client, info.Sender)
	if sender.Server == types.HiddenUserServer && info.SenderAlt.Server == types.DefaultUserServer {
		sender = info.SenderAlt.ToNonAD()
	}
	msg.From = sender.String()
	if name := b.savedName(ctx, client, sender); name != "" && !info.IsFromMe {
		msg.FromName = name
	}
	if img := evt.Message.GetImageMessage(); img != nil {
		msg.Media = encodeImage(img)
	}
	if mentioned := messageContext(evt.Message).GetMentionedJID(); len(mentioned) > 0 {
		names := map[string]string{}
		for _, raw := range mentioned {
			jid, err := types.ParseJID(raw)
			if err != nil {
				continue
			}
			if name := b.displayName(ctx, client, jid); name != "" {
				names[jid.User] = name
			}
		}
		msg.Text = replaceMentions(msg.Text, names)
	}
}

func (b *bridge) displayName(ctx context.Context, client *whatsmeow.Client, jid types.JID) string {
	resolved := b.resolve(ctx, client, jid)
	if client.Store.ID != nil && resolved.User == client.Store.ID.User {
		return "você"
	}
	info, err := client.Store.Contacts.GetContact(ctx, resolved)
	if err != nil || !info.Found {
		return ""
	}
	if name := savedName(info); name != "" {
		return name
	}
	return strings.TrimSpace(info.PushName)
}

func (b *bridge) participants(jid string) ([]Participant, error) {
	group, err := types.ParseJID(jid)
	if err != nil || group.Server != types.GroupServer {
		return nil, errors.New("conversa não é um grupo")
	}
	client := b.current()
	if !client.IsLoggedIn() {
		return nil, errNotConnected
	}
	ctx, cancel := context.WithTimeout(b.ctx, 30*time.Second)
	defer cancel()
	info, err := client.GetGroupInfo(ctx, group)
	if err != nil {
		return nil, err
	}
	result := make([]Participant, 0, len(info.Participants))
	for _, p := range info.Participants {
		name := b.displayName(ctx, client, p.JID)
		if name == "você" {
			continue
		}
		if name == "" && !p.PhoneNumber.IsEmpty() {
			name = "+" + p.PhoneNumber.User
		}
		if name == "" {
			name = p.JID.User
		}
		result = append(result, Participant{JID: p.JID.ToNonAD().String(), Name: name})
	}
	return result, nil
}

func (b *bridge) send(jid, text string, mentions []string) (Message, error) {
	if strings.TrimSpace(jid) == "" {
		return Message{}, errors.New("conversa não informada")
	}
	if strings.TrimSpace(text) == "" {
		return Message{}, errors.New("texto vazio")
	}
	target, err := types.ParseJID(jid)
	if err != nil {
		return Message{}, fmt.Errorf("conversa inválida: %w", err)
	}
	client := b.current()
	if !client.IsLoggedIn() || client.Store.ID == nil {
		return Message{}, errNotConnected
	}
	ctx, cancel := context.WithTimeout(b.ctx, 60*time.Second)
	defer cancel()
	content := &waE2E.Message{Conversation: proto.String(text)}
	if len(mentions) > 0 {
		content = &waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{
			Text:        proto.String(text),
			ContextInfo: &waE2E.ContextInfo{MentionedJID: mentions},
		}}
	}
	resp, err := client.SendMessage(ctx, target, content)
	if err != nil {
		return Message{}, err
	}
	return sentMessage(string(resp.ID), target, *client.Store.ID, text, resp.Timestamp), nil
}

const maxFileSize = 100 << 20

func (b *bridge) sendFile(jid, path, caption string) (Message, error) {
	target, err := types.ParseJID(jid)
	if err != nil || strings.TrimSpace(jid) == "" {
		return Message{}, errors.New("conversa inválida")
	}
	info, err := os.Stat(path)
	if err != nil {
		return Message{}, err
	}
	if info.Size() > maxFileSize {
		return Message{}, errors.New("arquivo maior que 100 MB")
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return Message{}, err
	}
	client := b.current()
	if !client.IsLoggedIn() || client.Store.ID == nil {
		return Message{}, errNotConnected
	}
	ctx, cancel := context.WithTimeout(b.ctx, 5*time.Minute)
	defer cancel()
	mimeType := fileMimeType(path, data)
	name := filepath.Base(path)
	image := strings.HasPrefix(mimeType, "image/") && mimeType != "image/svg+xml"
	mediaType := whatsmeow.MediaDocument
	if image {
		mediaType = whatsmeow.MediaImage
	}
	up, err := client.Upload(ctx, data, mediaType)
	if err != nil {
		return Message{}, err
	}
	var msg *waE2E.Message
	kind, text := KindDocument, name
	if image {
		kind, text = KindImage, caption
		msg = &waE2E.Message{ImageMessage: &waE2E.ImageMessage{
			Caption:       proto.String(caption),
			Mimetype:      proto.String(mimeType),
			URL:           &up.URL,
			DirectPath:    &up.DirectPath,
			MediaKey:      up.MediaKey,
			FileEncSHA256: up.FileEncSHA256,
			FileSHA256:    up.FileSHA256,
			FileLength:    &up.FileLength,
		}}
	} else {
		doc := &waE2E.DocumentMessage{
			FileName:      proto.String(name),
			Title:         proto.String(name),
			Mimetype:      proto.String(mimeType),
			URL:           &up.URL,
			DirectPath:    &up.DirectPath,
			MediaKey:      up.MediaKey,
			FileEncSHA256: up.FileEncSHA256,
			FileSHA256:    up.FileSHA256,
			FileLength:    &up.FileLength,
		}
		if caption != "" {
			doc.Caption = proto.String(caption)
			text = caption
			msg = &waE2E.Message{DocumentWithCaptionMessage: &waE2E.FutureProofMessage{
				Message: &waE2E.Message{DocumentMessage: doc},
			}}
		} else {
			msg = &waE2E.Message{DocumentMessage: doc}
		}
	}
	resp, err := client.SendMessage(ctx, target, msg)
	if err != nil {
		return Message{}, err
	}
	sent := sentMessage(string(resp.ID), target, *client.Store.ID, text, resp.Timestamp)
	sent.Kind = kind
	return sent, nil
}

func fileMimeType(path string, data []byte) string {
	if byExt := mime.TypeByExtension(strings.ToLower(filepath.Ext(path))); byExt != "" {
		return strings.SplitN(byExt, ";", 2)[0]
	}
	return strings.SplitN(http.DetectContentType(data), ";", 2)[0]
}

func (b *bridge) handleEvent(client *whatsmeow.Client, evt any) {
	if client != b.current() {
		return
	}
	switch e := evt.(type) {
	case *events.Connected:
		if client.Store.ID != nil {
			b.out.write(connectedEvent(client.Store.ID.ToNonAD().String()))
		}
	case *events.Message:
		if isRelevantMessage(e) {
			msg := messageFromEvent(e)
			b.enrich(client, e, &msg)
			b.out.write(messageEvent(msg, false))
		}
	case *events.Archive:
		ctx, cancel := context.WithTimeout(b.ctx, 10*time.Second)
		jid := b.resolve(ctx, client, e.JID)
		cancel()
		b.out.write(archivedEvent(jid.String(), e.Action.GetArchived()))
	case *events.HistorySync:
		b.handleHistory(client, e)
	case *events.LoggedOut:
		b.out.write(disconnectedEvent(ReasonSessionEnded))
		go b.resetSession(client)
	case *events.Disconnected:
		b.out.write(disconnectedEvent(ReasonConnectionLost))
	}
}

func (b *bridge) handleHistory(client *whatsmeow.Client, evt *events.HistorySync) {
	if evt.Data == nil {
		return
	}
	for _, conv := range evt.Data.GetConversations() {
		chat, err := types.ParseJID(conv.GetID())
		if err != nil {
			b.log.Warnf("conversa do histórico inválida %q: %v", conv.GetID(), err)
			continue
		}
		for _, item := range conv.GetMessages() {
			web := item.GetMessage()
			if web == nil {
				continue
			}
			parsed, err := client.ParseWebMessage(chat, web)
			if err != nil {
				b.log.Debugf("mensagem do histórico ignorada: %v", err)
				continue
			}
			if isRelevantMessage(parsed) {
				msg := messageFromEvent(parsed)
				b.enrich(client, parsed, &msg)
				b.out.write(messageEvent(msg, true))
			}
		}
	}
}

func (b *bridge) resetSession(old *whatsmeow.Client) {
	b.mu.Lock()
	defer b.mu.Unlock()
	if old != b.current() {
		return
	}
	old.Disconnect()
	if err := old.Store.Delete(b.ctx); err != nil {
		b.log.Errorf("falha ao apagar a sessão: %v", err)
	}
	b.client.Store(b.newClient(b.container.NewDevice()))
}

func (b *bridge) shutdown() {
	b.mu.Lock()
	defer b.mu.Unlock()
	if client := b.current(); client != nil {
		client.Disconnect()
	}
	if err := b.container.Close(); err != nil {
		b.log.Warnf("falha ao fechar o banco da sessão: %v", err)
	}
}
